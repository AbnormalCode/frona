use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use serde_json::Value;

use crate::core::error::AppError;
use crate::tool::{AgentTool, InferenceContext, ToolDefinition, ToolOutput};
use frona_harness::tools::files::image::{MAX_IMAGE_BYTES, is_supported_image, prepare_image};

use super::manager::McpManager;

// A result may contain several screenshots. Bound both work and retained bytes;
// bad/excess images become text placeholders without changing the MCP status.
const MAX_RESULT_IMAGES: usize = 8;
const MAX_RESULT_IMAGE_BYTES: usize = 20 * 1024 * 1024;

fn tool_output(result: rmcp::model::CallToolResult) -> ToolOutput {
    let mut text = Vec::new();
    let mut images = Vec::new();
    let mut image_count = 0;
    let mut image_bytes = 0;
    for content in result.content {
        match content {
            rmcp::model::ContentBlock::Text(value) => text.push(value.text),
            rmcp::model::ContentBlock::Image(value) => {
                image_count += 1;
                let prepared = (|| {
                    if image_count > MAX_RESULT_IMAGES {
                        return Err("MCP result exceeds the 8 image limit");
                    }
                    if !is_supported_image(&value.mime_type) {
                        return Err("unsupported image type; expected PNG, JPEG, GIF or WebP");
                    }
                    // Reject before base64 allocation, then check actual bytes
                    // in the shared validator (padding can change the size).
                    if value.data.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
                        return Err("image exceeds the 10 MiB size limit");
                    }
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(value.data)
                        .map_err(|_| "invalid base64 image data")?;
                    let image = prepare_image(&bytes, &value.mime_type)?;
                    if image_bytes + image.bytes.len() > MAX_RESULT_IMAGE_BYTES {
                        return Err("MCP result exceeds the 20 MiB image budget");
                    }
                    Ok(image)
                })();
                match prepared {
                    Ok(image) => {
                        image_bytes += image.bytes.len();
                        images.push(image);
                    }
                    Err(reason) => text.push(format!("[MCP image omitted: {reason}]")),
                }
            }
            _ => {}
        }
    }
    let text = text.join("\n");
    let output = if result.is_error.unwrap_or(false) {
        ToolOutput::error(text)
    } else {
        ToolOutput::text(text)
    };
    output.with_images(images)
}

pub struct McpTool {
    manager: Arc<McpManager>,
    owner_name: String,
    slug: String,
    cached_definitions: Vec<ToolDefinition>,
    /// Set when the server declared `resources` at initialize: the id the two
    /// synthetic resource tools below run against. A tools-only server grows
    /// nothing.
    resources_server_id: Option<String>,
}

/// Bare names of the synthetic tools that expose MCP resources. A server's own
/// tool of the same name always wins - these fill a gap, they never shadow.
pub const LIST_RESOURCES: &str = "list_resources";
pub const READ_RESOURCE: &str = "read_resource";

impl McpTool {
    pub fn new(manager: Arc<McpManager>, slug: &str, definitions: Vec<ToolDefinition>) -> Self {
        Self {
            manager,
            owner_name: format!("mcp__{slug}"),
            slug: slug.to_string(),
            cached_definitions: definitions,
            resources_server_id: None,
        }
    }

    /// Expose the server's resources as tools on the same namespace.
    pub fn with_resources(mut self, server_id: String) -> Self {
        self.resources_server_id = Some(server_id);
        self
    }

    fn has_real_tool(&self, name: &str) -> bool {
        let id = format!("mcp__{}__{name}", self.slug);
        self.cached_definitions.iter().any(|d| d.id == id)
    }
    async fn run_list_resources(&self) -> Result<ToolOutput, AppError> {
        let resources = self
            .manager
            .list_resources(self.resources_server_id.as_deref().unwrap_or_default())
            .await?;
        if resources.is_empty() {
            return Ok(ToolOutput::text("This server exposes no resources."));
        }
        let mut out = String::new();
        for r in &resources {
            out.push_str(&r.uri);
            let label = r.title.as_deref().unwrap_or(r.name.as_str());
            if !label.is_empty() {
                out.push_str(&format!("  {label}"));
            }
            if let Some(mime) = &r.mime_type {
                out.push_str(&format!("  [{mime}]"));
            }
            if let Some(desc) = &r.description {
                out.push_str(&format!("\n    {desc}"));
            }
            out.push('\n');
        }
        Ok(ToolOutput::text(out))
    }

    async fn run_read_resource(&self, arguments: Value) -> Result<ToolOutput, AppError> {
        let uri = arguments
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Tool("read_resource needs a `uri`".to_string()))?;

        let result = self
            .manager
            .read_resource(self.resources_server_id.as_deref().unwrap_or_default(), uri)
            .await?;
        let mut out = String::new();
        for content in &result.contents {
            match content {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                    out.push_str(text);
                    out.push('\n');
                }
                // Binary payloads are base64 in the wire format. Pasting that into a
                // transcript burns context and tells the agent nothing, so say what it
                // is instead.
                rmcp::model::ResourceContents::BlobResourceContents {
                    uri,
                    mime_type,
                    blob,
                    ..
                } => {
                    let kind = mime_type.as_deref().unwrap_or("application/octet-stream");
                    out.push_str(&format!(
                        "[binary resource {uri} ({kind}), {} base64 chars - not shown]\n",
                        blob.len()
                    ));
                }
                // `ResourceContents` is #[non_exhaustive]: a future rmcp may carry a
                // shape this build has no name for. Say so rather than drop it
                // silently, which would read as an empty resource.
                other => {
                    out.push_str(&format!(
                        "[resource content of a kind this build does not understand: {other:?}]\n"
                    ));
                }
            }
        }
        if out.is_empty() {
            out.push_str("(resource is empty)");
        }
        Ok(ToolOutput::text(out))
    }
}

/// The two synthetic tools that expose an MCP server's resources. A server's
/// own tool of the same name always wins, so a name `has_real_tool` already
/// claims is skipped rather than shadowed.
fn resource_definitions(slug: &str, has_real_tool: impl Fn(&str) -> bool) -> Vec<ToolDefinition> {
    let definition = |name: &str, description: &str, parameters: Value| ToolDefinition {
        id: format!("mcp__{slug}__{name}"),
        provider_id: format!("mcp:{slug}"),
        description: description.to_string(),
        parameters,
    };
    let mut defs = Vec::new();
    if !has_real_tool(LIST_RESOURCES) {
        defs.push(definition(
            LIST_RESOURCES,
            "List the resources this MCP server exposes, with each one's URI, name and \
             description. Resources hold content the server publishes for reading (guides, \
             schemas, files) rather than actions to call.",
            serde_json::json!({"type": "object", "properties": {}}),
        ));
    }
    if !has_real_tool(READ_RESOURCE) {
        defs.push(definition(
            READ_RESOURCE,
            "Read one resource from this MCP server by URI and return its contents. Use it \
             on any URI the server hands back (for example a `skill://` or `file://` link in \
             another tool's result).",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "Resource URI, exactly as the server gave it."
                    }
                },
                "required": ["uri"]
            }),
        ));
    }
    defs
}

#[async_trait]
impl AgentTool for McpTool {
    fn name(&self) -> &str {
        &self.owner_name
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut defs = self.cached_definitions.clone();

        // Resources are a first-class half of MCP that has no tool of its own, so a
        // server answering with a `uri` hands the agent something it otherwise cannot
        // dereference. Expose the two calls as tools on the same server namespace, so
        // policy, the registry and the bridge all treat them like any other MCP tool.
        if self.resources_server_id.is_some() {
            defs.extend(resource_definitions(&self.slug, |name| {
                self.has_real_tool(name)
            }));
        }
        defs
    }

    async fn execute(
        &self,
        tool_name: &str,
        arguments: Value,
        ctx: &InferenceContext,
    ) -> Result<ToolOutput, AppError> {
        let prefix = format!("{}__", self.owner_name);
        let bare_name = tool_name.strip_prefix(&prefix).unwrap_or(tool_name);

        // Mirror `definitions`: only handle these when the server did not supply a
        // real tool of the same name, or we would shadow it.
        if self.resources_server_id.is_some() && !self.has_real_tool(bare_name) {
            match bare_name {
                LIST_RESOURCES => return self.run_list_resources().await,
                READ_RESOURCE => return self.run_read_resource(arguments).await,
                _ => {}
            }
        }

        let server_id = self
            .manager
            .server_for_tool(&ctx.user.id, tool_name)
            .await
            .ok_or_else(|| {
                AppError::Tool(format!("no running MCP server exposes tool {tool_name}"))
            })?;

        let result = self.manager.call(&server_id, bare_name, arguments).await?;

        Ok(tool_output(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_tools_are_offered_when_the_server_has_none_of_that_name() {
        let defs = resource_definitions("docs", |_| false);
        let ids: Vec<_> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["mcp__docs__list_resources", "mcp__docs__read_resource"]
        );
        assert!(defs.iter().all(|d| d.provider_id == "mcp:docs"));
        assert_eq!(defs[1].parameters["required"], serde_json::json!(["uri"]));
    }

    /// A server that ships its own `read_resource` keeps it: the synthetic one
    /// would otherwise shadow a tool whose behaviour the agent was told about.
    #[test]
    fn a_servers_own_tool_of_the_same_name_is_never_shadowed() {
        let defs = resource_definitions("docs", |name| name == READ_RESOURCE);
        let ids: Vec<_> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["mcp__docs__list_resources"]);

        let none = resource_definitions("docs", |_| true);
        assert!(none.is_empty());
    }
    use rmcp::model::{CallToolResult, ContentBlock};

    fn png() -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        buffer.into_inner()
    }

    fn image(bytes: &[u8]) -> ContentBlock {
        ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(bytes),
            "image/png",
        )
    }

    #[test]
    fn text_and_image_results_keep_text_order_and_error_status() {
        let png = png();
        for is_error in [false, true] {
            let mut result = CallToolResult::success(vec![
                ContentBlock::text("before"),
                image(&png),
                ContentBlock::text("after"),
            ]);
            result.is_error = Some(is_error);
            let output = tool_output(result);
            assert_eq!(output.is_success(), !is_error);
            assert_eq!(output.text_content(), "before\nafter");
            assert_eq!(output.images().len(), 1);
            assert_eq!(output.images()[0].bytes, png);
            assert_eq!(output.images()[0].media_type, "image/png");
        }
    }

    #[test]
    fn image_only_text_only_and_empty_results_remain_explicit() {
        let output = tool_output(CallToolResult::success(vec![image(&png())]));
        assert!(output.is_success());
        assert_eq!(output.images().len(), 1);
        assert!(output.text_content().is_empty());
        let output = tool_output(CallToolResult::success(vec![
            ContentBlock::text("one"),
            ContentBlock::text("two"),
        ]));
        assert_eq!(output.text_content(), "one\ntwo");
        assert!(output.images().is_empty());
        let output = tool_output(CallToolResult::success(vec![]));
        assert!(output.is_success());
        assert!(output.text_content().is_empty());
        assert!(output.images().is_empty());
    }

    #[test]
    fn unusable_images_get_ordered_informative_placeholders() {
        let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        for (data, mime, reason) in [
            ("invalid!".into(), "image/png", "invalid base64"),
            (encode(b"not an image"), "image/png", "could not decode"),
            (encode(b"<svg/>"), "image/svg+xml", "unsupported image type"),
            (encode(&png()), "image/jpeg", "MIME type does not match"),
            (
                "A".repeat(MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1),
                "image/png",
                "10 MiB",
            ),
        ] {
            let mut result = CallToolResult::success(vec![
                ContentBlock::text("before"),
                ContentBlock::image(data, mime),
                ContentBlock::text("after"),
            ]);
            result.is_error = Some(true);
            let output = tool_output(result);
            assert!(!output.is_success());
            assert!(output.images().is_empty());
            assert!(
                output
                    .text_content()
                    .starts_with("before\n[MCP image omitted:")
            );
            assert!(output.text_content().contains(reason));
            assert!(output.text_content().ends_with("]\nafter"));
        }
    }

    #[test]
    fn excessive_image_count_is_reported_without_dropping_text() {
        let png = png();
        let mut content: Vec<_> = (0..9).map(|_| image(&png)).collect();
        content.push(ContentBlock::text("after"));
        let output = tool_output(CallToolResult::success(content));
        assert!(output.is_success());
        assert_eq!(output.images().len(), 8);
        assert!(output.text_content().contains("8 image limit"));
        assert!(output.text_content().ends_with("after"));
    }

    #[test]
    fn aggregate_image_bytes_are_bounded() {
        // Valid PNG with trailing bytes: each image fits its individual limit,
        // but retaining all three would exceed the whole-result budget.
        let mut bytes = png();
        bytes.resize(7 * 1024 * 1024, 0);
        let output = tool_output(CallToolResult::success(
            (0..3).map(|_| image(&bytes)).collect(),
        ));
        assert_eq!(output.images().len(), 2);
        assert!(output.text_content().contains("20 MiB image budget"));
        assert!(output.is_success());
    }
}
