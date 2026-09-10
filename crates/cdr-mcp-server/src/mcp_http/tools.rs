use std::borrow::Cow;
use std::sync::Arc;

use rmcp::model::{JsonObject, MetaObject, Tool, ToolAnnotations};
use serde::Deserialize;

const PYTHON_TOOL_INVENTORY: &str = include_str!("../../assets/python_tool_inventory.json");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InventoryTool {
    name: String,
    title: Option<String>,
    description: Option<String>,
    input_schema: JsonObject,
    output_schema: Option<JsonObject>,
    annotations: Option<ToolAnnotations>,
    #[serde(rename = "_meta")]
    meta: Option<MetaObject>,
}

pub(super) fn definitions() -> Result<Vec<Tool>, serde_json::Error> {
    let inventory: Vec<InventoryTool> = serde_json::from_str(PYTHON_TOOL_INVENTORY)?;
    Ok(inventory.into_iter().map(into_tool).collect())
}

fn into_tool(source: InventoryTool) -> Tool {
    let mut tool = Tool::new_with_raw(
        source.name,
        source.description.map(Cow::Owned),
        Arc::new(source.input_schema),
    );
    tool.title = source.title;
    tool.output_schema = source.output_schema.map(Arc::new);
    tool.annotations = source.annotations;
    tool.meta = source.meta;
    tool
}
