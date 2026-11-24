//! Foreground tool surface for the knowledge service. Reading a page is the general `read`
//! tool (pages are self-describing `.md` files). Tool definitions live in
//! `resources/prompts/tools/pkm/<name>.md` (loaded by the `#[agent_tool]` macro).

use std::sync::Arc;

use serde_json::Value;

use frona_derive::agent_tool;

use crate::auth::user_service::UserService;
use crate::core::error::AppError;
use crate::tool::{InferenceContext, ToolOutput, active_chat, str_arg, str_list_arg};
use frona_text::prompt::PromptLoader;

use super::ontology::OntologyManager;
use super::search::MemorySearch;
use super::storage::PkmStorage;
use super::vault::VaultScope;
use crate::db::repo::pkm::PkmRepo;
use frona_pkm::ontology::GraphDirection;

pub fn all(
    repo: Arc<PkmRepo>,
    storage: PkmStorage,
    ontology: OntologyManager,
    prompts: PromptLoader,
    user_service: UserService,
) -> Vec<Arc<dyn crate::tool::AgentTool>> {
    let vault = VaultResolver {
        storage,
        user_service,
    };
    vec![
        Arc::new(RememberTool {
            repo: repo.clone(),
            prompts: prompts.clone(),
        }),
        Arc::new(SearchTool {
            search: MemorySearch::new(repo.clone(), ontology.clone()),
            prompts: prompts.clone(),
            vault: vault.clone(),
        }),
        Arc::new(CitePageTool {
            repo,
            prompts: prompts.clone(),
            vault: vault.clone(),
        }),
        Arc::new(GraphGetTool {
            ontology: ontology.clone(),
            prompts: prompts.clone(),
        }),
        Arc::new(GraphSparqlTool { ontology, prompts }),
    ]
}

pub struct GraphSparqlTool {
    ontology: OntologyManager,
    prompts: PromptLoader,
}

#[agent_tool(name = "memory_graph_sparql", dir = "pkm")]
impl GraphSparqlTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let query = arg(&arguments, "query")?;
        let result = self.ontology.query_graph(&ctx.user.id, query, 200).await?;
        Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?))
    }
}

pub struct GraphGetTool {
    ontology: OntologyManager,
    prompts: PromptLoader,
}

#[agent_tool(name = "memory_graph_get", dir = "pkm")]
impl GraphGetTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let path = arg(&arguments, "path")?.trim_end_matches(".md");
        let direction = match arguments
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("both")
        {
            "outgoing" => GraphDirection::Outgoing,
            "incoming" => GraphDirection::Incoming,
            "both" => GraphDirection::Both,
            value => {
                return Err(AppError::Validation(format!(
                    "invalid 'direction' value '{value}'"
                )));
            }
        };
        let relation = str_arg(&arguments, "relation");
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 100) as usize;
        let Some(entity) = self
            .ontology
            .graph_entity(&ctx.user.id, path, direction, relation, limit)
            .await?
        else {
            return Ok(ToolOutput::text(format!(
                "No reasoned entity exists at '{path}'."
            )));
        };
        Ok(ToolOutput::text(serde_json::to_string_pretty(&entity)?))
    }
}

/// The two dependencies [`VaultScope::resolve`] needs, as one collaborator.
///
/// `memory_search` and `memory_cite` both work in page paths, so both have to know where
/// this user's files live - and neither wants `PkmStorage` or `UserService` for anything
/// else. Carried loose, they were two fields apiece whose reason for existing was a call
/// spelled out identically in both, which is one edit away from two answers to "which
/// directory is the vault?".
#[derive(Clone)]
struct VaultResolver {
    storage: PkmStorage,
    user_service: UserService,
}

impl VaultResolver {
    async fn for_caller(&self, ctx: &InferenceContext) -> Result<VaultScope, AppError> {
        VaultScope::resolve(
            &self.user_service,
            &self.storage,
            &ctx.user.id,
            &ctx.user.handle,
        )
        .await
    }
}

/// [`str_arg`] plus this surface's policy: a missing argument is a validation error the
/// tool loop reports back, rather than a message the tool composes itself. The graph
/// tools take one scalar each; the page tools take `str_list_arg` batches.
fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    str_arg(args, key).ok_or_else(|| AppError::Validation(format!("missing '{key}'")))
}

pub struct RememberTool {
    repo: Arc<PkmRepo>,
    prompts: PromptLoader,
}

#[agent_tool(name = "memory_remember", dir = "pkm")]
impl RememberTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        // The unit of *storage* is one sentence - consolidation grounds and files each
        // separately - but the unit of *calling* needn't be: a conversation that
        // surfaced a host, a port and a password used to cost three tool turns.
        let contents = str_list_arg(&arguments, "content", "contents");
        if contents.is_empty() {
            return Err(AppError::Validation("missing 'content'".into()));
        }
        let chat = active_chat(ctx)?;
        // A private-memory agent's note is scoped to the agent, so it never reaches
        // the user's other agents or the consolidation that builds the vault.
        let private_agent = ctx.agent.private_memory.then_some(ctx.agent.id.as_str());
        for content in &contents {
            self.repo
                .remember(&ctx.user.id, &chat.id, content, private_agent)
                .await?;
        }
        Ok(ToolOutput::text(format!(
            "Remembered:\n{}",
            contents
                .iter()
                .map(|c| format!("- {c}"))
                .collect::<Vec<_>>()
                .join("\n")
        )))
    }
}

pub struct SearchTool {
    search: MemorySearch,
    prompts: PromptLoader,
    vault: VaultResolver,
}

#[agent_tool(name = "memory_search", dir = "pkm")]
impl SearchTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let queries = str_list_arg(&arguments, "query", "queries");
        if queries.is_empty() {
            return Err(AppError::Validation("missing 'query'".into()));
        }
        let vault = self.vault.for_caller(ctx).await?;
        // A lone query keeps the bare result every caller already parses; several
        // come back keyed by the query that produced them.
        if let [only] = queries.as_slice() {
            let result = self.search.execute(&ctx.user.id, only, &vault).await?;
            return Ok(ToolOutput::text(serde_json::to_string_pretty(&result)?));
        }
        let mut results = Vec::with_capacity(queries.len());
        for query in &queries {
            let result = self.search.execute(&ctx.user.id, query, &vault).await?;
            results.push(serde_json::json!({ "query": query, "result": result }));
        }
        Ok(ToolOutput::text(serde_json::to_string_pretty(&results)?))
    }
}

pub struct CitePageTool {
    repo: Arc<PkmRepo>,
    prompts: PromptLoader,
    vault: VaultResolver,
}

#[agent_tool(name = "memory_cite", dir = "pkm")]
impl CitePageTool {
    async fn execute(
        &self,
        _tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let raws = str_list_arg(&arguments, "path", "paths");
        if raws.is_empty() {
            return Err(AppError::Validation("missing 'path'".into()));
        }
        let vault = self.vault.for_caller(ctx).await?;
        let mut lines = Vec::with_capacity(raws.len());
        for raw in &raws {
            // Accept the vault-relative path memory_search returned (or an absolute
            // one); fall back to a bare page path for resilience.
            let page_path = vault
                .page_from_any(raw)
                .unwrap_or_else(|| raw.trim_end_matches(".md").to_string());
            // One bad path mustn't void the others: the rest of the batch is still
            // bookkeeping the agent asked for.
            lines.push(match self.repo.bump_entity_use(&ctx.user.id, &page_path).await {
                Ok(n) => format!("Cited '{page_path}' (total uses: {n})."),
                Err(AppError::Validation(msg)) => format!(
                    "Couldn't cite '{raw}': {msg}. Use the absolute path returned by memory_search."
                ),
                Err(e) => return Err(e),
            });
        }
        Ok(ToolOutput::text(lines.join("\n")))
    }
}
