mod arguments;
mod inventory;
mod operation;
mod special;

use std::future::Future;
use std::pin::Pin;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::broker::BridgeBroker;
use crate::mcp_http::{DispatchOutput, ToolCallContext, ToolDispatcher};

pub const PRODUCTION_CONNECTOR_RESOURCE: &str = "https://simdorei.duckdns.org/mcp";

#[derive(Clone)]
pub struct BrokerToolDispatcher {
    pub(super) broker: BridgeBroker,
    pub(super) resource_url: String,
}

impl BrokerToolDispatcher {
    #[must_use]
    pub fn new(broker: BridgeBroker, resource_url: String) -> Self {
        Self {
            broker,
            resource_url,
        }
    }

    async fn dispatch_inner(
        &self,
        context: ToolCallContext,
        tool: String,
        arguments: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        if let Some(result) = self
            .dispatch_special(&context, &tool, arguments.clone())
            .await
        {
            return result;
        }
        let operation = operation::parse(&tool, arguments)?;
        let output = self
            .broker
            .project_operation(
                &context.session,
                &context.subject,
                context.request_id,
                operation,
                context.cancellation,
            )
            .await
            .map_err(|error| error.to_string())?;
        operation::dispatch_output(output)
    }
}

impl ToolDispatcher for BrokerToolDispatcher {
    fn dispatch(
        &self,
        context: ToolCallContext,
        tool: String,
        arguments: Map<String, Value>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchOutput, String>> + Send>> {
        let dispatcher = self.clone();
        Box::pin(async move { dispatcher.dispatch_inner(context, tool, arguments).await })
    }
}

fn structured(value: impl Serialize) -> Result<DispatchOutput, String> {
    serde_json::to_value(value)
        .map(DispatchOutput::Structured)
        .map_err(|error| format!("failed to serialize tool output: {error}"))
}

fn arguments<T: serde::de::DeserializeOwned>(value: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(value))
        .map_err(|error| format!("invalid tool arguments: {error}"))
}
