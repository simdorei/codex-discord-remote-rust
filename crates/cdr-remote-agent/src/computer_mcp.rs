//! Explicit, operator-enabled stdio MCP access to the existing computer engine.

mod catalog;
mod coordinator;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::io::{self, BufRead, Write};

use cdr_core::Validate;
use cdr_remote_protocol::output::ComputerOutput;
use cdr_remote_protocol::request::ComputerRequest;
use serde_json::{Map, Value, json};

use crate::computer::{
    ComputerAccessMode, ComputerController, ComputerError, new_computer_controller,
};

const MAX_INPUT_BYTES: usize = 128 * 1024;
const MAX_REQUEST_IDS: usize = 16_384;
const MAX_REQUEST_ID_BYTES: usize = 1024 * 1024;
const CURRENT_PROTOCOL: &str = "2025-06-18";

/// Serve MCP messages until EOF, then stop this process's computer controller.
/// Desktop resources are acquired only for a validated `tools/call` request.
pub fn run_stdio() -> Result<(), String> {
    let mut controller = None;
    let mut coordinator = coordinator::Coordinator::new();
    let result = serve(io::stdin().lock(), io::stdout().lock(), |request| {
        coordinator
            .execute(request, |request| execute_request(&mut controller, request))
            .map_err(|error| error.to_string())
    });
    let cleanup = controller.map_or(Ok(()), |active| {
        coordinator
            .execute(&ComputerRequest::ComputerStop, |request| {
                active.execute(request)
            })
            .map(|_| ())
            .map_err(|error| format!("Computer cleanup failed: {error}"))
    });
    match (result, cleanup) {
        (Err(primary), Err(cleanup)) => Err(format!("{primary}; {cleanup}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn execute_request(
    controller: &mut Option<ComputerController>,
    request: &ComputerRequest,
) -> Result<ComputerOutput, ComputerError> {
    if controller.is_none() {
        *controller = Some(new_computer_controller(ComputerAccessMode::Device)?);
    }
    controller
        .as_ref()
        .ok_or(ComputerError::State)?
        .execute(request)
}

fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut execute: impl FnMut(&ComputerRequest) -> Result<ComputerOutput, String>,
) -> Result<(), String> {
    let mut session = Session::default();
    loop {
        let response = match read_frame(&mut input).map_err(|error| error.to_string())? {
            Frame::Eof => return Ok(()),
            Frame::Incomplete => {
                write_message(
                    &mut output,
                    &rpc_error(&Value::Null, -32700, "Incomplete frame; newline required."),
                )?;
                return Err(
                    "MCP stream ended before the final frame's newline; nothing executed.".into(),
                );
            }
            Frame::TooLarge => {
                write_message(
                    &mut output,
                    &rpc_error(&Value::Null, -32600, "Input exceeds 128 KiB."),
                )?;
                return Err("MCP input exceeded the 128 KiB limit; connection closed.".into());
            }
            Frame::Message(bytes) => match serde_json::from_slice(&bytes) {
                Ok(message) => session.handle(&message, &mut execute),
                Err(_) => Some(rpc_error(&Value::Null, -32700, "Invalid JSON.")),
            },
        };
        if let Some(response) = response {
            write_message(&mut output, &response)?;
        }
    }
}

enum Frame {
    Eof,
    Incomplete,
    Message(Vec<u8>),
    TooLarge,
}

fn read_frame(input: &mut impl BufRead) -> io::Result<Frame> {
    let mut bytes = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok(if bytes.is_empty() {
                Frame::Eof
            } else {
                Frame::Incomplete
            });
        }
        let used = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if used > MAX_INPUT_BYTES - bytes.len() {
            return Ok(Frame::TooLarge);
        }
        bytes.extend_from_slice(&available[..used]);
        let complete = available[used - 1] == b'\n';
        input.consume(used);
        if complete {
            return Ok(Frame::Message(bytes));
        }
    }
}

fn write_message(output: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *output, message).map_err(|error| error.to_string())?;
    output
        .write_all(b"\n")
        .and_then(|()| output.flush())
        .map_err(|error| error.to_string())
}

#[derive(Default)]
struct Session {
    initialized: bool,
    ready: bool,
    protocol: &'static str,
    request_ids: HashSet<String>,
    request_id_bytes: usize,
}

type RpcResult = Result<Value, (i64, String)>;

impl Session {
    fn handle(
        &mut self,
        message: &Value,
        execute: &mut impl FnMut(&ComputerRequest) -> Result<ComputerOutput, String>,
    ) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(rpc_error(
                &Value::Null,
                -32600,
                "Expected one JSON-RPC object.",
            ));
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || object
                .keys()
                .any(|key| !["jsonrpc", "id", "method", "params"].contains(&key.as_str()))
        {
            return Some(rpc_error(
                &Value::Null,
                -32600,
                "Invalid JSON-RPC envelope.",
            ));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(rpc_error(&Value::Null, -32600, "Missing request method."));
        };
        let empty = Map::new();
        let params = object.get("params").map_or(Some(&empty), Value::as_object);
        let Some(id) = object.get("id") else {
            if method == "notifications/initialized"
                && self.initialized
                && params.is_some_and(|params| params_match(params, &[]))
            {
                self.ready = true;
            }
            return None;
        };
        if !(id.is_string() || id.as_i64().is_some() || id.as_u64().is_some()) {
            return Some(rpc_error(
                &Value::Null,
                -32600,
                "Request ID must be a string or integer.",
            ));
        }
        let key = id.to_string();
        if self.request_ids.contains(&key) {
            return Some(rpc_error(
                id,
                -32600,
                "Duplicate request ID; nothing executed.",
            ));
        }
        if self.request_ids.len() >= MAX_REQUEST_IDS
            || key.len() > MAX_REQUEST_ID_BYTES - self.request_id_bytes
        {
            return Some(rpc_error(
                id,
                -32000,
                "Request ID budget exhausted; reconnect.",
            ));
        }
        self.request_id_bytes += key.len();
        self.request_ids.insert(key);
        let result = params
            .ok_or_else(|| invalid_params("Params must be an object."))
            .and_then(|params| self.dispatch(method, params, execute));
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
            Err((code, message)) => rpc_error(id, code, &message),
        })
    }

    fn dispatch(
        &mut self,
        method: &str,
        params: &Map<String, Value>,
        execute: &mut impl FnMut(&ComputerRequest) -> Result<ComputerOutput, String>,
    ) -> RpcResult {
        if method == "initialize" {
            return self.initialize(params);
        }
        if method == "ping" && params_match(params, &[]) {
            return Ok(json!({}));
        }
        if !self.ready {
            return Err((
                -32002,
                "Complete initialize and notifications/initialized first.".into(),
            ));
        }
        match method {
            "ping" => Err(invalid_params("Unknown ping parameters.")),
            "tools/list" if params_match(params, &[]) => Ok(json!({"tools":catalog::tools()})),
            "tools/list" => Err(invalid_params("This tool catalog is not paginated.")),
            "tools/call" => {
                let request = catalog::request(params).map_err(|error| (-32602, error))?;
                let result = execute(&request)
                    .and_then(|value| tool_result(value, self.protocol == CURRENT_PROTOCOL));
                Ok(result.unwrap_or_else(|message| {
                    json!({
                        "content":[{"type":"text", "text":message}], "isError":true
                    })
                }))
            }
            _ => Err((-32601, "Method not found.".into())),
        }
    }

    fn initialize(&mut self, params: &Map<String, Value>) -> RpcResult {
        if self.initialized {
            return Err((-32600, "This connection is already initialized.".into()));
        }
        let version = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let client = params.get("clientInfo").and_then(Value::as_object);
        if !params_match(params, &["protocolVersion", "capabilities", "clientInfo"])
            || version.is_none()
            || !params.get("capabilities").is_some_and(Value::is_object)
            || !client.is_some_and(|client| {
                ["name", "version"].iter().all(|key| {
                    client
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                })
            })
        {
            return Err(invalid_params("Invalid MCP initialize parameters."));
        }
        self.protocol = match version {
            Some("2024-11-05") => "2024-11-05",
            Some("2025-03-26") => "2025-03-26",
            _ => CURRENT_PROTOCOL,
        };
        self.initialized = true;
        Ok(json!({
            "protocolVersion": self.protocol,
            "capabilities": {"tools":{"listChanged":false}},
            "serverInfo": {"name":"cdr_computer", "version":env!("CARGO_PKG_VERSION")},
            "instructions": catalog::INSTRUCTIONS,
        }))
    }
}

fn params_match(params: &Map<String, Value>, names: &[&str]) -> bool {
    params
        .keys()
        .all(|key| key == "_meta" || names.contains(&key.as_str()))
        && params.get("_meta").is_none_or(Value::is_object)
}

fn invalid_params(message: &str) -> (i64, String) {
    (-32602, message.into())
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
}

fn tool_result(output: ComputerOutput, structured: bool) -> Result<Value, String> {
    output
        .validate()
        .map_err(|error| format!("Invalid computer output: {error}"))?;
    let (metadata, image) = match output {
        ComputerOutput::ComputerScreenshot {
            observation_id,
            window,
            data_base64,
            ..
        } => (
            json!({"kind":"computer_screenshot", "observation_id":observation_id,
                "window":window, "media_type":"image/png"}),
            Some(json!({"type":"image", "data":data_base64, "mimeType":"image/png"})),
        ),
        other => (
            serde_json::to_value(other).map_err(|error| error.to_string())?,
            None,
        ),
    };
    let mut content = vec![
        json!({"type":"text", "text":serde_json::to_string(&metadata)
        .map_err(|error| error.to_string())?}),
    ];
    content.extend(image);
    let mut result = json!({"content":content, "isError":false});
    if structured {
        result["structuredContent"] = metadata;
    }
    Ok(result)
}
