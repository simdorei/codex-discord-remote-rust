use std::sync::Arc;

use axum::http::request::Parts;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::auth::AuthorizedAccess;
use super::{DispatchOutput, ToolCallContext, ToolDispatcher, tools};
use crate::capability::required_scopes;

#[derive(Clone)]
pub(super) struct McpGatewayHandler {
    dispatcher: Arc<dyn ToolDispatcher>,
    tools: Arc<Vec<Tool>>,
}

impl McpGatewayHandler {
    pub fn new(dispatcher: Arc<dyn ToolDispatcher>) -> Result<Self, serde_json::Error> {
        Ok(Self {
            dispatcher,
            tools: Arc::new(tools::definitions()?),
        })
    }

    fn visible_error(message: impl Into<String>) -> CallToolResponse {
        CallToolResult::error(vec![ContentBlock::text(message.into())]).into()
    }
}

impl ServerHandler for McpGatewayHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "simdorei-local-project",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Operate the selected local project through its connected agent.")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let Some(required) = required_scopes(&name) else {
            return Err(ErrorData::invalid_params("Unknown tool.", None));
        };
        let Some(parts) = context.extensions.get::<Parts>() else {
            return Ok(Self::visible_error("OAuth request context is unavailable."));
        };
        let Some(access) = parts.extensions.get::<AuthorizedAccess>() else {
            return Ok(Self::visible_error("OAuth identity is unavailable."));
        };
        if let Some(missing) = required
            .iter()
            .find(|scope| !access.0.scopes.iter().any(|granted| granted == **scope))
        {
            return Ok(Self::visible_error(format!(
                "OAuth scope {missing} is required."
            )));
        }
        let Some(session) = context
            .meta
            .get("openai/session")
            .and_then(serde_json::Value::as_str)
            .filter(|session| !session.is_empty())
        else {
            return Ok(Self::visible_error(
                "ChatGPT did not provide conversation session metadata.",
            ));
        };
        let Some(subject) = access.0.subject.as_deref() else {
            return Ok(Self::visible_error("OAuth subject is unavailable."));
        };
        let call_context = ToolCallContext {
            session: session.to_owned(),
            subject: principal(subject, &access.0.client_id),
            request_id: request_id(session, &context),
            cancellation: context.ct.clone(),
        };
        let arguments = request.arguments.unwrap_or_default();
        match self
            .dispatcher
            .dispatch(call_context, name, arguments)
            .await
        {
            Ok(DispatchOutput::Structured(value)) => Ok(CallToolResult::structured(value).into()),
            Ok(DispatchOutput::Image { data, mime_type }) => {
                Ok(CallToolResult::success(vec![ContentBlock::image(data, mime_type)]).into())
            }
            Err(message) => Ok(Self::visible_error(message)),
        }
    }
}

fn principal(subject: &str, client_id: &str) -> String {
    hex_digest(format!("{}:{subject}{client_id}", subject.len()).as_bytes())
}

fn request_id(session: &str, context: &RequestContext<RoleServer>) -> String {
    hex_digest(
        format!(
            "{}:{session}:{:?}:{}",
            session.len(),
            context.id,
            Uuid::new_v4()
        )
        .as_bytes(),
    )
}

fn hex_digest(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}
