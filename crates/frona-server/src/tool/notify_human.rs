use serde_json::Value;

use crate::core::error::AppError;
use crate::credential::vault::service::VaultService;
use frona_text::prompt::PromptLoader;

use crate::inference::hitl::{Hitl, HitlOutcome, HitlRequest, HitlResponse};
use crate::inference::tool_call::ToolStatus;
use frona_derive::agent_tool;

use super::browser::active_profile;
use super::{InferenceContext, ToolOutput, active_chat};
use crate::core::config::TakeoverMethod;

/// The takeover link for `method`. The debugger needs a saved login to attach
/// to (its route is keyed by credential), so without one it falls back to the
/// live view rather than handing the person no link at all.
fn takeover_url(
    method: TakeoverMethod,
    public_base_url: &str,
    profile: &str,
    first_credential_id: Option<&str>,
) -> String {
    match (method, first_credential_id) {
        (TakeoverMethod::Debugger, Some(credential_id)) => {
            format!("/api/browser/debugger/{credential_id}")
        }
        _ => {
            let query: String = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("profile", profile)
                .finish();
            format!("{public_base_url}/browser?{query}")
        }
    }
}

pub struct NotifyHumanTool {
    vault_service: VaultService,
    prompts: PromptLoader,
    public_base_url: String,
    takeover: TakeoverMethod,
}

impl NotifyHumanTool {
    pub fn new(
        vault_service: VaultService,
        prompts: PromptLoader,
        public_base_url: String,
        takeover: TakeoverMethod,
    ) -> Self {
        Self {
            vault_service,
            prompts,
            public_base_url,
            takeover,
        }
    }

    /// Where the person is sent to take over the browser, per the configured
    /// method. Kept in the HITL request's `debugger_url` field so stored
    /// takeovers still parse.
    async fn takeover_url(&self, user_id: &str) -> String {
        let profile = active_profile(&self.vault_service, user_id).await;
        let first_credential = match self.takeover {
            TakeoverMethod::Debugger => self
                .vault_service
                .list_credentials(user_id)
                .await
                .ok()
                .and_then(|creds| creds.into_iter().next())
                .map(|c| c.id),
            TakeoverMethod::LiveView => None,
        };
        takeover_url(
            self.takeover,
            &self.public_base_url,
            &profile,
            first_credential.as_deref(),
        )
    }

    fn url_for_chat(&self, chat_id: &str) -> String {
        format!("{}/chat?id={}", self.public_base_url, chat_id)
    }
}

#[agent_tool(files("ask_user_question", "request_user_takeover"))]
impl NotifyHumanTool {
    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let chat = active_chat(ctx)?;
        match tool_name {
            "request_user_takeover" => {
                let reason = arguments
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("User intervention needed")
                    .to_string();
                let debugger_url = self.takeover_url(&ctx.user.id).await;

                Ok(ToolOutput::text("").with_hitl(Hitl {
                    prompt: if debugger_url.is_empty() {
                        reason.clone()
                    } else {
                        format!("{reason}\n\nTake over: {debugger_url}")
                    },
                    url: self.url_for_chat(&chat.id),
                    request: HitlRequest::Takeover {
                        reason,
                        debugger_url,
                    },
                    status: ToolStatus::Pending,
                    response: None,
                    delivery: None,
                }))
            }
            "ask_user_question" => {
                let question = arguments
                    .get("question")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                let options: Vec<String> = arguments
                    .get("options")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                Ok(ToolOutput::text("").with_hitl(Hitl {
                    prompt: question,
                    url: self.url_for_chat(&chat.id),
                    request: HitlRequest::Question { options },
                    status: ToolStatus::Pending,
                    response: None,
                    delivery: None,
                }))
            }
            _ => Err(AppError::Tool(format!(
                "Unknown notify_human sub-tool: {tool_name}"
            ))),
        }
    }

    async fn on_resume(
        &self,
        tool_name: &str,
        _request: &HitlRequest,
        response: HitlResponse,
        _ctx: &InferenceContext,
    ) -> Result<HitlOutcome, AppError> {
        let HitlResponse::Choice(text) = response else {
            return Err(AppError::Validation(format!(
                "notify_human::{tool_name} expected HitlResponse::Choice"
            )));
        };
        match tool_name {
            "ask_user_question" => Ok(HitlOutcome::Resolved(text)),
            "request_user_takeover" => {
                let resolved = if text.is_empty() {
                    "Human completed the takeover.".to_string()
                } else {
                    text
                };
                Ok(HitlOutcome::Resolved(resolved))
            }
            _ => Err(AppError::Tool(format!(
                "Unknown notify_human sub-tool: {tool_name}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://frona.example.com";

    #[test]
    fn live_view_links_to_the_page_for_the_active_profile() {
        let url = takeover_url(TakeoverMethod::LiveView, BASE, "google", Some("cred-1"));
        assert_eq!(url, "https://frona.example.com/browser?profile=google");
    }

    #[test]
    fn the_profile_is_url_encoded() {
        let url = takeover_url(TakeoverMethod::LiveView, BASE, "a b&c", None);
        assert_eq!(url, "https://frona.example.com/browser?profile=a+b%26c");
    }

    #[test]
    fn debugger_links_to_the_credentials_proxy_route() {
        let url = takeover_url(TakeoverMethod::Debugger, BASE, "google", Some("cred-1"));
        assert_eq!(url, "/api/browser/debugger/cred-1");
    }

    /// The debugger route is keyed by a saved login. A person with none must
    /// still be given a way in, not an empty link.
    #[test]
    fn debugger_falls_back_to_the_live_view_without_a_saved_login() {
        let url = takeover_url(TakeoverMethod::Debugger, BASE, "default", None);
        assert_eq!(url, "https://frona.example.com/browser?profile=default");
    }

    #[test]
    fn the_default_method_is_the_live_view() {
        assert_eq!(TakeoverMethod::default(), TakeoverMethod::LiveView);
    }
}
