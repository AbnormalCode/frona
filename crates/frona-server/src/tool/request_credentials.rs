use serde_json::Value;

use crate::core::Principal;
use crate::core::error::AppError;
use crate::credential::vault::models::{BindingScope, GrantDuration};
use crate::credential::vault::service::VaultService;
use crate::inference::hitl::{
    CredentialRequest, Hitl, HitlOutcome, HitlRequest, HitlResponse, SkippedCredential, VaultGrant,
};
use crate::inference::tool_call::ToolStatus;
use frona_text::prompt::PromptLoader;

use frona_derive::agent_tool;

use super::{InferenceContext, ToolOutput, active_chat};

pub struct RequestCredentialsTool {
    vault_service: VaultService,
    prompts: PromptLoader,
    public_base_url: String,
}

impl RequestCredentialsTool {
    pub fn new(
        vault_service: VaultService,
        prompts: PromptLoader,
        public_base_url: String,
    ) -> Self {
        Self {
            vault_service,
            prompts,
            public_base_url,
        }
    }

    fn scope_for(
        grant_duration: &GrantDuration,
        chat_id: &str,
    ) -> (BindingScope, Option<chrono::DateTime<chrono::Utc>>) {
        match grant_duration {
            GrantDuration::Once => (
                BindingScope::Chat {
                    chat_id: chat_id.to_string(),
                },
                None,
            ),
            GrantDuration::Hours(h) => (
                BindingScope::Durable,
                Some(chrono::Utc::now() + chrono::Duration::hours(*h as i64)),
            ),
            GrantDuration::Days(d) => (
                BindingScope::Durable,
                Some(chrono::Utc::now() + chrono::Duration::days(*d as i64)),
            ),
            GrantDuration::Permanent => (BindingScope::Durable, None),
        }
    }

    /// Persist one approved grant (durable grant record when it outlives the
    /// chat, the binding, the audit entry) and return the env vars it projects.
    #[allow(clippy::too_many_arguments)]
    async fn apply_grant(
        &self,
        ctx: &InferenceContext,
        principal: &Principal,
        chat_id: &str,
        query: &str,
        connection_id: &str,
        vault_item_id: &str,
        grant_duration: &GrantDuration,
        target: &crate::credential::vault::models::CredentialTarget,
        reason: &str,
    ) -> Result<Vec<String>, AppError> {
        let secret = self
            .vault_service
            .get_secret(&ctx.user.id, connection_id, vault_item_id)
            .await?;

        if !matches!(grant_duration, GrantDuration::Once) {
            self.vault_service
                .create_grant(
                    &ctx.user.id,
                    principal.clone(),
                    connection_id,
                    vault_item_id,
                    query,
                    grant_duration,
                )
                .await?;
        }

        let (scope, expires_at) = Self::scope_for(grant_duration, chat_id);
        self.vault_service
            .create_binding(
                &ctx.user.id,
                principal.clone(),
                query,
                connection_id,
                vault_item_id,
                target.clone(),
                scope,
                expires_at,
            )
            .await?;
        self.vault_service
            .log_access(
                &ctx.user.id,
                principal.clone(),
                chat_id,
                connection_id,
                vault_item_id,
                None,
                query,
                reason,
            )
            .await?;

        let env_vars = crate::credential::vault::service::project_target(&secret, target)?;
        Ok(env_vars.into_iter().map(|(k, _)| k).collect())
    }

    /// Normalize the tool arguments into the credentials the agent wants.
    /// Accepts a single `query` string and/or a `queries` array whose elements
    /// are plain strings or `{query, label}` objects, so the agent can ask for
    /// every secret an API needs in one call. Duplicates collapse to one slot.
    fn parse_requested(arguments: &Value) -> Result<Vec<CredentialRequest>, AppError> {
        let mut items: Vec<CredentialRequest> = Vec::new();
        let mut push = |query: &str, label: Option<String>| {
            let query = query.trim();
            if query.is_empty() || items.iter().any(|i| i.query == query) {
                return;
            }
            items.push(CredentialRequest {
                query: query.to_string(),
                label,
            });
        };

        if let Some(arr) = arguments.get("queries").and_then(|v| v.as_array()) {
            for el in arr {
                if let Some(s) = el.as_str() {
                    push(s, None);
                } else if let Some(obj) = el.as_object()
                    && let Some(q) = obj.get("query").and_then(|v| v.as_str())
                {
                    let label = obj
                        .get("label")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    push(q, label);
                }
            }
        }
        if let Some(q) = arguments.get("query").and_then(|v| v.as_str()) {
            push(q, None);
        }

        if items.is_empty() {
            return Err(AppError::Validation(
                "Missing required parameter: provide `query` or a non-empty `queries` array".into(),
            ));
        }
        Ok(items)
    }

    fn batch_prompt(items: &[CredentialRequest], reason: &str) -> String {
        if let [only] = items {
            return format!(
                "Allow access to credential matching '{}'?\n\n{reason}",
                only.query
            );
        }
        let lines: Vec<String> = items
            .iter()
            .map(|i| match &i.label {
                Some(label) => format!("• {label} — matching '{}'", i.query),
                None => format!("• matching '{}'", i.query),
            })
            .collect();
        format!(
            "Allow access to {} credentials? Each one can be skipped.\n\n{reason}\n\n{}",
            items.len(),
            lines.join("\n"),
        )
    }

    /// Env var names a binding projects, after authorizing it and logging the
    /// access under `owner_id` (the user whose vault the secret is read from).
    async fn load_binding(
        &self,
        owner_id: &str,
        principal: &Principal,
        binding: &crate::credential::vault::models::PrincipalCredentialBinding,
        chat_id: &str,
        query: &str,
        reason: &str,
    ) -> Result<Vec<String>, AppError> {
        let secret = self
            .vault_service
            .resolve_binding(owner_id, principal, binding, Some(chat_id))
            .await?;
        self.vault_service
            .log_access(
                owner_id,
                principal.clone(),
                chat_id,
                &binding.connection_id,
                &binding.vault_item_id,
                None,
                query,
                reason,
            )
            .await?;
        let env_vars = crate::credential::vault::service::project_target(&secret, &binding.target)?;
        Ok(env_vars.into_iter().map(|(k, _)| k).collect())
    }

    /// One-line account of what the user declined, for the agent.
    fn skipped_summary(skipped: &[SkippedCredential]) -> String {
        skipped
            .iter()
            .map(
                |s| match s.reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                    Some(reason) => format!("'{}' skipped by the user: {reason}", s.query),
                    None => format!("'{}' skipped by the user (no reason given)", s.query),
                },
            )
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[agent_tool]
impl RequestCredentialsTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let chat = active_chat(ctx)?;
        let reason = arguments
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Credential access requested")
            .to_string();
        let force = arguments
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let requested = Self::parse_requested(&arguments)?;
        let principal = Principal::agent(ctx.agent.id.clone());

        // Items an existing grant (the chat's own, or the owner's on a shared
        // agent with credential delegation) already covers load immediately;
        // only the rest need the user. Re-runs and partly granted batches never
        // re-ask for secrets the chat already has.
        let mut var_names: Vec<String> = Vec::new();
        let mut pending: Vec<CredentialRequest> = Vec::new();
        for item in requested {
            if !force {
                if let Some(binding) = self
                    .vault_service
                    .find_binding(&ctx.user.id, &principal, &item.query, Some(&chat.id))
                    .await?
                {
                    var_names.extend(
                        self.load_binding(
                            &ctx.user.id,
                            &principal,
                            &binding,
                            &chat.id,
                            &item.query,
                            &reason,
                        )
                        .await?,
                    );
                    continue;
                }
                if let Some(owner_id) = ctx.delegated_credential_owner.as_deref()
                    && let Some(binding) = self
                        .vault_service
                        .find_binding(owner_id, &principal, &item.query, None)
                        .await?
                {
                    var_names.extend(
                        self.load_binding(
                            owner_id,
                            &principal,
                            &binding,
                            &chat.id,
                            &item.query,
                            &reason,
                        )
                        .await?,
                    );
                    continue;
                }
            }
            pending.push(item);
        }

        if pending.is_empty() {
            return Ok(ToolOutput::text(format!(
                "Credentials approved for command startup: {}. Each command resolves current values.",
                var_names.join(", ")
            )));
        }

        // A lone request keeps the original single-slot shape so existing
        // clients and stored rows are unaffected.
        let prompt = Self::batch_prompt(&pending, &reason);
        let request = if let [only] = pending.as_slice()
            && only.label.is_none()
        {
            HitlRequest::Credential {
                query: only.query.clone(),
                reason,
            }
        } else {
            HitlRequest::Credentials {
                items: pending,
                reason,
            }
        };
        Ok(ToolOutput::text("").with_hitl(Hitl {
            prompt,
            url: format!("{}/chat?id={}", self.public_base_url, chat.id),
            request,
            status: ToolStatus::Pending,
            response: None,
            delivery: None,
        }))
    }

    async fn on_resume(
        &self,
        _tool_name: &str,
        request: &HitlRequest,
        response: HitlResponse,
        ctx: &InferenceContext,
    ) -> Result<HitlOutcome, AppError> {
        let chat = active_chat(ctx)?;
        let reason = match request {
            HitlRequest::Credential { reason, .. } | HitlRequest::Credentials { reason, .. } => {
                reason.clone()
            }
            _ => {
                return Err(AppError::Validation(
                    "request_credentials on_resume: expected Credential(s) request".into(),
                ));
            }
        };
        let principal = Principal::agent(ctx.agent.id.clone());

        match response {
            HitlResponse::Vault(VaultGrant::GrantedMany { grants, skipped }) => {
                let mut var_names: Vec<String> = Vec::new();
                for grant in grants {
                    var_names.extend(
                        self.apply_grant(
                            ctx,
                            &principal,
                            &chat.id,
                            &grant.query,
                            &grant.connection_id,
                            &grant.vault_item_id,
                            &grant.grant_duration,
                            &grant.target,
                            &reason,
                        )
                        .await?,
                    );
                }

                if var_names.is_empty() {
                    return Ok(HitlOutcome::Denied(format!(
                        "No credentials were provided: {}.",
                        Self::skipped_summary(&skipped),
                    )));
                }
                let mut text = format!(
                    "Credentials approved for command startup: {}. Each command resolves current values.",
                    var_names.join(", "),
                );
                if !skipped.is_empty() {
                    text.push_str(&format!(
                        " Not provided — {}. Do not ask for these again unless the user does.",
                        Self::skipped_summary(&skipped),
                    ));
                }
                Ok(HitlOutcome::Resolved(text))
            }
            HitlResponse::Vault(VaultGrant::Granted {
                connection_id,
                vault_item_id,
                grant_duration,
                target,
            }) => {
                // Single-slot path: only a `Credential` request names the query
                // this grant resolves, but a one-slot `Credentials` also lands here.
                let query = match request {
                    HitlRequest::Credential { query, .. } => query.clone(),
                    HitlRequest::Credentials { items, .. } if items.len() == 1 => {
                        items[0].query.clone()
                    }
                    _ => {
                        return Err(AppError::Validation(
                            "request_credentials on_resume: single Granted response needs a single-slot request".into(),
                        ));
                    }
                };
                let var_names = self
                    .apply_grant(
                        ctx,
                        &principal,
                        &chat.id,
                        &query,
                        &connection_id,
                        &vault_item_id,
                        &grant_duration,
                        &target,
                        &reason,
                    )
                    .await?;
                Ok(HitlOutcome::Resolved(format!(
                    "Credentials approved for command startup: {}. Each command resolves current values.",
                    var_names.join(", "),
                )))
            }
            HitlResponse::Vault(VaultGrant::Denied) => {
                let label = match request {
                    HitlRequest::Credential { query, .. } => query.clone(),
                    HitlRequest::Credentials { items, .. } => items
                        .iter()
                        .map(|i| i.query.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => "requested credentials".to_string(),
                };
                Ok(HitlOutcome::Denied(format!(
                    "User denied access to credentials for: {label}.",
                )))
            }
            _ => Err(AppError::Validation(
                "request_credentials on_resume: expected Vault response".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_requested_merges_query_and_queries_without_duplicates() {
        let items = RequestCredentialsTool::parse_requested(&json!({
            "queries": ["app key", {"query": "user key", "label": " User key "}, "app key", "  "],
            "query": "user key",
        }))
        .unwrap();
        let got: Vec<_> = items
            .iter()
            .map(|i| (i.query.as_str(), i.label.as_deref()))
            .collect();
        assert_eq!(got, vec![("app key", None), ("user key", Some("User key"))]);
    }

    #[test]
    fn parse_requested_needs_something_to_ask_for() {
        assert!(RequestCredentialsTool::parse_requested(&json!({"reason": "x"})).is_err());
        assert!(RequestCredentialsTool::parse_requested(&json!({"queries": []})).is_err());
    }

    #[test]
    fn skipped_summary_relays_reasons_and_notes_their_absence() {
        let text = RequestCredentialsTool::skipped_summary(&[
            SkippedCredential {
                query: "a".into(),
                reason: Some("not set up yet".into()),
            },
            SkippedCredential {
                query: "b".into(),
                reason: Some("  ".into()),
            },
        ]);
        assert_eq!(
            text,
            "'a' skipped by the user: not set up yet; 'b' skipped by the user (no reason given)"
        );
    }
}
