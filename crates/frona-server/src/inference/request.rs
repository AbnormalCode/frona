use rig_core::completion::Message as RigMessage;
use tokio_util::sync::CancellationToken;

use crate::agent::models::Agent;
use crate::agent::task::models::Task;
use crate::auth::User;
use crate::chat::broadcast::EventSender;
use crate::chat::models::Chat;
use crate::core::error::AppError;
use crate::tool::registry::AgentToolRegistry;

use crate::inference::{tool_call::TaskEvent, usage::UsageService};
use frona_harness::inference::ModelGroup;

use crate::chat::message::models::Reasoning;

pub struct InferenceContext {
    pub user: User,
    pub agent: Agent,
    /// Handle of the user who OWNS `agent`. Equal to `user.handle` for an owned
    /// agent; differs when the agent was shared with the running user, so
    /// definition/execution-scoped lookups (skills, agent workspace, sandbox
    /// policy) resolve under the owner. Runner-scoped concerns (vault, memory,
    /// ephemeral token) keep using `user`.
    pub agent_owner_handle: crate::core::Handle,
    /// When this is a shared agent whose owner opted into credential
    /// delegation, the owner's user id - the account whose agent credential
    /// bindings this run may use. `None` for owned agents or shares without
    /// delegation.
    pub delegated_credential_owner: Option<String>,
    /// The originating chat, or `None` for a **detached** inference (background /
    /// sync passes that have no chat - e.g. the PKM sync investigator). Chat-scoped
    /// tools reach it via [`active_chat`], which refuses cleanly when it's `None`.
    pub chat: Option<Chat>,
    pub task: Option<Task>,
    pub event_tx: EventSender,
    /// Resolved filesystem paths for files shared in this chat (from message attachments).
    pub file_paths: Vec<String>,
    pub shutdown_token: CancellationToken,
    /// User-initiated cancellation token - tools should check/use this to abort early.
    pub cancel_token: CancellationToken,
}

impl InferenceContext {
    pub fn agent_context_scope(&self) -> crate::agent::context::AgentContextScope<'_> {
        crate::agent::context::AgentContextScope {
            user: &self.user,
            agent: &self.agent,
            space_id: self.chat.as_ref().and_then(|chat| chat.space_id.as_deref()),
            task: self.task.as_ref(),
        }
    }

    /// The normal chat-bound context. Callers keep passing a `Chat`; it's wrapped
    /// `Some` so every existing chat-path caller is unaffected.
    pub fn new(
        user: User,
        agent: Agent,
        chat: Chat,
        event_tx: EventSender,
        shutdown_token: CancellationToken,
        cancel_token: CancellationToken,
    ) -> Self {
        // Default to the runner as owner; `with_agent_owner_handle` overrides
        // this for shared agents. Correct for the common owned-agent case.
        let agent_owner_handle = user.handle.clone();
        Self {
            user,
            agent,
            agent_owner_handle,
            delegated_credential_owner: None,
            chat: Some(chat),
            task: None,
            event_tx,
            file_paths: Vec::new(),
            shutdown_token,
            cancel_token,
        }
    }

    /// A detached (chatless) context for background/sync inference. There's no chat
    /// to stream to, so it wires its own [`EventSender::noop`]. Chat-scoped tools
    /// invoked here refuse via [`active_chat`].
    pub fn new_detached(
        user: User,
        agent: Agent,
        shutdown_token: CancellationToken,
        cancel_token: CancellationToken,
    ) -> Self {
        Self {
            // Detached runs have no share context: the runner is the owner.
            agent_owner_handle: user.handle.clone(),
            delegated_credential_owner: None,
            user,
            agent,
            chat: None,
            task: None,
            event_tx: EventSender::noop(),
            file_paths: Vec::new(),
            shutdown_token,
            cancel_token,
        }
    }

    pub fn with_task(mut self, task: Task) -> Self {
        self.task = Some(task);
        self
    }

    /// Override the agent-owner handle (used when the agent was shared with the
    /// running user, so their runs resolve the owner's skills/workspace/policy).
    pub fn with_agent_owner_handle(mut self, handle: crate::core::Handle) -> Self {
        self.agent_owner_handle = handle;
        self
    }

    /// Mark that this run may use the given owner's delegated agent credentials.
    pub fn with_delegated_credential_owner(mut self, owner_id: Option<String>) -> Self {
        self.delegated_credential_owner = owner_id;
        self
    }
}

/// Borrow the active chat, or refuse - the choke point for chat-scoped tools on a
/// detached context. A **free function** (not a method) deliberately: the erroring
/// `AppError` policy doesn't belong on the context data type.
pub fn active_chat(ctx: &InferenceContext) -> Result<&Chat, AppError> {
    ctx.chat
        .as_ref()
        .ok_or_else(|| AppError::Tool("this tool requires an active chat".into()))
}

pub struct InferenceRequest {
    pub input_token_estimate: Option<frona_harness::inference::context::InputTokenEstimate>,
    pub model_group: ModelGroup,
    pub system_prompt: String,
    pub history: Vec<RigMessage>,
    pub tool_registry: AgentToolRegistry,
    pub ctx: InferenceContext,
    pub cancel_token: CancellationToken,
    pub chat_service: crate::chat::service::ChatService,
    pub message_id: String,
    pub usage_service: UsageService,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum InferenceResponse {
    Completed {
        text: String,
        attachments: Vec<crate::storage::Attachment>,
        lifecycle_event: Option<TaskEvent>,
        reasoning: Option<Reasoning>,
    },
    /// Server shutdown leaves canonical work resumable on startup. Core work
    /// is incomplete; this is distinct from user cancellation or task failure.
    Shutdown {
        text: String,
        attachments: Vec<crate::storage::Attachment>,
        reasoning: Option<Reasoning>,
    },
    /// Work ended without protocol completion. Partial output is displayable,
    /// but never an accepted refinement or a successful task completion.
    Incomplete {
        reason: frona_harness::ExecutionStop,
        text: String,
        attachments: Vec<crate::storage::Attachment>,
        reasoning: Option<crate::chat::message::models::Reasoning>,
    },
    Cancelled(String),
    ExternalToolPending {
        turn_text: String,
        tool_calls: Vec<crate::inference::tool_call::ToolCallResponse>,
        system_prompt: Option<String>,
    },
    /// The harness already wrote the response for this turn - `finalize`
    /// should leave the placeholder agent message alone. Used by the
    /// `Command` dispatch path (`/clear`, `/compact`, `/title`, etc.) which
    /// either completes the placeholder inline or wipes it as a side effect.
    Handled,
}
