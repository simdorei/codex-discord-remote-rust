use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::RpcErrorPayload;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Integer(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ServerRequestOccurrence([u8; 16]);

impl ServerRequestOccurrence {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    fn random() -> Self {
        Self(Uuid::new_v4().into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRequest {
    pub id: RequestId,
    pub occurrence: ServerRequestOccurrence,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug)]
pub(crate) enum IncomingMessage {
    ServerRequest(ServerRequest),
    Response {
        id: RequestId,
        result: Result<Value, RpcErrorPayload>,
    },
    Notification(Notification),
    Ignored,
}

pub(crate) fn classify(value: Value) -> Result<IncomingMessage, serde_json::Error> {
    let Value::Object(mut object) = value else {
        return Ok(IncomingMessage::Ignored);
    };
    let id = take_id(&mut object)?;
    let method = object
        .remove("method")
        .and_then(|value| value.as_str().map(str::to_owned));
    if let (Some(id), Some(method)) = (id.clone(), method.clone())
        && !object.contains_key("result")
        && !object.contains_key("error")
    {
        return Ok(IncomingMessage::ServerRequest(ServerRequest {
            id,
            occurrence: ServerRequestOccurrence::random(),
            method,
            params: object.remove("params").unwrap_or_else(empty_object),
        }));
    }
    if let Some(id) = id {
        if let Some(error) = object.remove("error") {
            return Ok(IncomingMessage::Response {
                id,
                result: Err(serde_json::from_value(error)?),
            });
        }
        return Ok(IncomingMessage::Response {
            id,
            result: Ok(object.remove("result").unwrap_or_else(empty_object)),
        });
    }
    Ok(method.map_or(IncomingMessage::Ignored, |method| {
        IncomingMessage::Notification(Notification {
            method,
            params: object.remove("params").unwrap_or_else(empty_object),
        })
    }))
}

pub(crate) fn request_value(id: &RequestId, method: &str, params: &Value) -> Value {
    serde_json::json!({"id": id, "method": method, "params": params})
}

pub(crate) fn notification_value(method: &str, params: &Value) -> Value {
    serde_json::json!({"method": method, "params": params})
}

pub(crate) fn response_value(id: &RequestId, result: &Value) -> Value {
    serde_json::json!({"id": id, "result": result})
}

pub(crate) fn error_value(id: &RequestId, error: &RpcErrorPayload) -> Value {
    serde_json::json!({"id": id, "error": error})
}

fn take_id(object: &mut Map<String, Value>) -> Result<Option<RequestId>, serde_json::Error> {
    object.remove("id").map(serde_json::from_value).transpose()
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}
