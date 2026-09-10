use serde_json::{Map, Value, json};

use super::arguments::{
    DirectoryArgs, ListFilesArgs, ReadFileArgs, SelectDeviceArgs, SelectProjectArgs, WriteFileArgs,
};
use super::inventory::capability_inventory;
use super::{BrokerToolDispatcher, PRODUCTION_CONNECTOR_RESOURCE, arguments, structured};
use crate::mcp_http::{DispatchOutput, ToolCallContext};

impl BrokerToolDispatcher {
    pub(super) async fn dispatch_special(
        &self,
        context: &ToolCallContext,
        tool: &str,
        values: Map<String, Value>,
    ) -> Option<Result<DispatchOutput, String>> {
        let result = match tool {
            "capability_inventory" => empty(&values).map(|()| capability_inventory()),
            "list_devices" => self.list_devices(values).await,
            "select_project" => self.select_project(context, values).await,
            "select_device" => self.select_device(context, values).await,
            "set_working_directory" => self.set_directory(context, values).await,
            "device_info" => self.device_info(context, values).await,
            "project_info" => self.project_info(context, values).await,
            "list_project_files" => self.list_files(context, values).await,
            "read_project_file" | "file_read_slice" => self.read_file(context, values).await,
            "write_project_file" => self.write_file(context, values).await,
            _ => return None,
        };
        Some(result)
    }

    async fn list_devices(&self, values: Map<String, Value>) -> Result<DispatchOutput, String> {
        empty(&values)?;
        structured(json!({"devices": self.broker.list_devices().await}))
    }

    async fn select_project(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: SelectProjectArgs = arguments(values)?;
        self.require_connector(&input.connector_resource)?;
        let output = self
            .broker
            .select_project(&context.session, &context.subject, &input.project_scope)
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn select_device(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: SelectDeviceArgs = arguments(values)?;
        self.require_connector(&input.connector_resource)?;
        let output = self
            .broker
            .select_device(
                &context.session,
                &context.subject,
                &input.device_id,
                &input.working_directory,
            )
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn set_directory(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: DirectoryArgs = arguments(values)?;
        let output = self
            .broker
            .set_working_directory(&context.session, &context.subject, &input.working_directory)
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn device_info(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        empty(&values)?;
        let output = self
            .broker
            .device_info(&context.session, &context.subject)
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn project_info(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        empty(&values)?;
        let output = self
            .broker
            .project_info(&context.session, &context.subject)
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn list_files(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: ListFilesArgs = arguments(values)?;
        let output = self
            .broker
            .list_files(
                &context.session,
                &context.subject,
                input.pattern,
                input.limit,
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn read_file(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: ReadFileArgs = arguments(values)?;
        let output = self
            .broker
            .read_file(
                &context.session,
                &context.subject,
                context.request_id.clone(),
                input.path,
                input.start_line,
                input.max_lines,
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    async fn write_file(
        &self,
        context: &ToolCallContext,
        values: Map<String, Value>,
    ) -> Result<DispatchOutput, String> {
        let input: WriteFileArgs = arguments(values)?;
        let output = self
            .broker
            .write_file(
                &context.session,
                &context.subject,
                context.request_id.clone(),
                input.path,
                input.content,
                input.expected_sha256,
                context.cancellation.clone(),
            )
            .await
            .map_err(|error| error.to_string())?;
        structured(output)
    }

    fn require_connector(&self, requested: &str) -> Result<(), String> {
        if self.resource_url == PRODUCTION_CONNECTOR_RESOURCE
            && requested == PRODUCTION_CONNECTOR_RESOURCE
        {
            Ok(())
        } else {
            Err("OAuth connector mismatch: select 'Simdorei Local Project Oauth' and retry.".into())
        }
    }
}

fn empty(values: &Map<String, Value>) -> Result<(), String> {
    if values.is_empty() {
        Ok(())
    } else {
        Err("this tool does not accept arguments".into())
    }
}
