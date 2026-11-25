use super::*;
use crate::core::{Handle, error::AppError};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

const ENV_PREFIX: &str = "FRONA_";

const EXCLUDED_ENV_VARS: &[&str] = &[
    "FRONA_CONFIG",
    "FRONA_LOG_CONFIG",
    "FRONA_LOG_LEVEL",
    "FRONA_SERVER_DATA_DIR",
];

pub struct LoadedConfig {
    pub(super) path: PathBuf,
    pub(super) revision: String,
    pub(super) defaults: Config,
    pub config: Config,
    pub models: Option<crate::inference::config::ModelRegistryConfig>,
}

impl ConfigService {
    /// Read a startup snapshot without a database. A missing file uses defaults
    /// and environment overrides; malformed or unreadable input is an error.
    pub fn load(path: impl AsRef<Path>) -> Result<LoadedConfig, AppError> {
        Self::load_with_env(path, std::env::vars().collect())
    }

    pub(super) fn load_with_env(
        path: impl AsRef<Path>,
        env: HashMap<String, String>,
    ) -> Result<LoadedConfig, AppError> {
        let config_path = path.as_ref().to_path_buf();
        let bytes = super::document::read_file(&config_path)?;
        let revision = super::document::revision(&bytes);
        let yaml_content = if bytes.is_empty() {
            None
        } else {
            Some(
                String::from_utf8(bytes)
                    .map_err(|error| AppError::Validation(error.to_string()))?,
            )
        };

        let defaults = Self::resolve_yaml_with_env(None, env.clone())?;
        let mut config = Self::resolve_yaml_with_env(yaml_content.as_deref(), env)?;
        resolve_server_timezone(&mut config.server);

        let models = if !config.models.is_empty() || !config.providers.is_empty() {
            Some(crate::inference::config::ModelRegistryConfig {
                providers: config.providers.clone().into_iter().collect(),
                models: config.models.clone().into_iter().collect(),
                skip_auto_discover: false,
            })
        } else {
            None
        };

        if yaml_content.is_some() {
            tracing::info!(path = %config_path.display(), "Loaded config from YAML");
        } else {
            tracing::info!("No config file found, using defaults and env vars");
        }

        if let Ok(mut v) = serde_json::to_value(&config) {
            redact_config_for_log(&mut v);
            tracing::debug!(
                "Effective config:\n{}",
                serde_json::to_string_pretty(&v).unwrap_or_default()
            );
        }

        Ok(LoadedConfig {
            config,
            defaults,
            models,
            path: config_path,
            revision,
        })
    }

    /// Resolve the settings view with the same defaults and environment precedence
    /// as startup, without adding environment values to the authoring document.
    pub(crate) fn resolve_document_with_env(
        document: &serde_json::Value,
        env: HashMap<String, String>,
    ) -> Result<Config, AppError> {
        let yaml = serde_yaml::to_string(document)
            .map_err(|error| AppError::Validation(error.to_string()))?;
        Self::resolve_yaml_with_env(Some(&yaml), env)
    }

    fn resolve_yaml_with_env(
        yaml_content: Option<&str>,
        env: HashMap<String, String>,
    ) -> Result<Config, AppError> {
        let data_dir = env
            .get("FRONA_SERVER_DATA_DIR")
            .cloned()
            .unwrap_or_else(|| "data".into());

        let mut builder = config::Config::builder()
            .set_default("database.path", format!("{data_dir}/db"))
            .unwrap()
            .set_default("storage.data_dir", data_dir.clone())
            .unwrap()
            .set_default("storage.skills_dir", format!("{data_dir}/skills"))
            .unwrap()
            .set_default("storage.cache_dir", format!("{data_dir}/system/cache"))
            .unwrap()
            .set_default("storage.ontology_dir", format!("{data_dir}/ontology"))
            .unwrap();

        if let Some(content) = yaml_content {
            let expanded = lower_legacy_model_keys(&expand_config_env_vars(content)?)?;
            builder =
                builder.add_source(config::File::from_str(&expanded, config::FileFormat::Yaml));
        }

        // FRONA_BROWSER_WS_URL -> browser__ws_url -> browser.ws_url
        let frona_env: HashMap<String, String> = env
            .into_iter()
            .filter(|(k, _)| k.starts_with(ENV_PREFIX) && !EXCLUDED_ENV_VARS.contains(&k.as_str()))
            .map(|(k, v)| {
                let stripped = k[ENV_PREFIX.len()..].to_lowercase();
                let mapped = match stripped.find('_') {
                    Some(pos) => format!("{}__{}", &stripped[..pos], &stripped[pos + 1..]),
                    None => stripped,
                };
                (mapped, v)
            })
            .collect();

        builder = builder.add_source(
            config::Environment::default()
                .source(Some(frona_env))
                .separator("__")
                .try_parsing(true),
        );

        let built = builder
            .build()
            .map_err(|error| AppError::Validation(error.to_string()))?;

        let mut config: Config = built
            .try_deserialize()
            .map_err(|error| AppError::Validation(error.to_string()))?;

        // Preserve provider credential source references for runtime precedence.
        // Other configuration fields retain the existing expansion behavior.
        if let Some(content) = yaml_content {
            let raw: serde_yaml::Value = serde_yaml::from_str(content)
                .map_err(|error| AppError::Validation(error.to_string()))?;
            if let Some(providers) = raw.get("providers").and_then(serde_yaml::Value::as_mapping) {
                for (name, provider) in providers {
                    let Some(name) = name.as_str() else { continue };
                    let Some(key) = provider.get("api_key").and_then(serde_yaml::Value::as_str)
                    else {
                        continue;
                    };
                    if key.starts_with("${") && key.ends_with('}') {
                        let handle = Handle::try_new(name)?;
                        if let Some(entry) = config.providers.get_mut(&handle) {
                            entry.api_key = Some(key.to_string());
                        }
                    }
                }
            }
        }

        Ok(config)
    }
}

/// Model-level keys the pre-rebuild config accepted for OpenRouter. The harness
/// schema has no such fields (and rejects unknown ones), but every request body
/// takes native `extra_params`, so a config written for the old layout keeps
/// working by moving them there: `provider_routing` is sent as OpenRouter's
/// `provider` object, `route` as `route`. `prompt_caching` has no equivalent
/// (the harness does not place cache breakpoints for OpenRouter) and is dropped.
const LEGACY_MODEL_KEYS: [&str; 3] = ["provider_routing", "route", "prompt_caching"];

pub(super) fn lower_legacy_model_keys(yaml: &str) -> Result<String, AppError> {
    if !LEGACY_MODEL_KEYS
        .iter()
        .chain(&["api_version"])
        .any(|key| yaml.contains(key))
    {
        return Ok(yaml.to_string());
    }
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|error| AppError::Validation(error.to_string()))?;
    let mut changed = false;
    if let Some(models) = document
        .get_mut("models")
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        for (_, group) in models.iter_mut() {
            changed |= lower_group(group);
        }
    }
    if let Some(providers) = document
        .get_mut("providers")
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        for (handle, provider) in providers.iter_mut() {
            changed |= lower_azure_api_version(handle, provider);
        }
    }
    if !changed {
        return Ok(yaml.to_string());
    }
    serde_yaml::to_string(&document).map_err(|error| AppError::Validation(error.to_string()))
}

/// `api_version` was a provider-wide field that only Azure read ("ignored by other
/// providers"). Azure's now lives in the provider attributes as
/// `azure_api_version`; the harness rejects the old key as an unknown attribute,
/// which for any *other* provider would stop the server starting over a setting it
/// never used - so there it is dropped.
fn lower_azure_api_version(handle: &serde_yaml::Value, provider: &mut serde_yaml::Value) -> bool {
    use serde_yaml::Value;
    let Some(map) = provider.as_mapping_mut() else {
        return false;
    };
    let is_azure = map
        .get("provider")
        .and_then(Value::as_str)
        .or_else(|| handle.as_str())
        == Some("azure");
    let Some(version) = map.remove("api_version") else {
        return false;
    };
    if !is_azure {
        if !version.is_null() {
            tracing::warn!(
                provider = handle.as_str().unwrap_or("?"),
                "Ignoring providers.*.api_version: only Azure OpenAI used it"
            );
        }
        return true;
    }
    map.entry(Value::String("azure_api_version".into()))
        .or_insert(version);
    true
}

fn lower_group(group: &mut serde_yaml::Value) -> bool {
    use serde_yaml::Value;
    let Some(map) = group.as_mapping_mut() else {
        return false;
    };
    let mut changed = false;
    let mut lowered: Vec<(&str, Value)> = Vec::new();
    for key in LEGACY_MODEL_KEYS {
        if let Some(value) = map.remove(key) {
            changed = true;
            match key {
                "provider_routing" => lowered.push(("provider", value)),
                "route" => lowered.push(("route", value)),
                _ => tracing::warn!(
                    "Ignoring models.*.prompt_caching: OpenRouter cache breakpoints are not sent"
                ),
            }
        }
    }
    if !lowered.is_empty() {
        let extra = map
            .entry(Value::String("extra_params".into()))
            .or_insert_with(|| Value::Mapping(Default::default()));
        if let Some(extra) = extra.as_mapping_mut() {
            for (name, value) in lowered {
                // An explicit extra_params entry wins over the legacy key.
                extra.entry(Value::String(name.into())).or_insert(value);
            }
        }
    }
    if let Some(fallbacks) = map.get_mut("fallbacks").and_then(Value::as_sequence_mut) {
        for fallback in fallbacks {
            changed |= lower_group(fallback);
        }
    }
    changed
}
