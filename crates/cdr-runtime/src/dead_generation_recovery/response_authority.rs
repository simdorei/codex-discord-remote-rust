use super::{RuntimeDeadGenerationFence, mutation_held};
use cdr_app_server::{AppServerError, ServerRequest};
use cdr_store::mutation_attempt::response::{self, Scope};
use serde_json::{Value, json};

fn request_value(request: &ServerRequest) -> Value {
    json!({"id":request.id,"occurrence":request.occurrence,
        "method":request.method,"params":request.params})
}

fn scope<'a>(
    fence: &'a RuntimeDeadGenerationFence,
    owner: (&'a str, u64),
    request: &'a Value,
) -> Result<Scope<'a>, AppServerError> {
    Ok(Scope {
        runtime: &fence.runtime_id,
        resident: owner.0,
        generation: i64::try_from(owner.1).map_err(mutation_held)?,
        request,
    })
}

pub(super) fn capture(
    fence: &RuntimeDeadGenerationFence,
    owner: (&str, u64),
    request: &ServerRequest,
) -> Result<Value, AppServerError> {
    response::capture(
        &fence.mirror_db,
        &scope(fence, owner, &request_value(request))?,
    )
    .map_err(mutation_held)
}

pub(super) fn begin(
    fence: &RuntimeDeadGenerationFence,
    owner: (&str, u64),
    request: &ServerRequest,
    authority: &Value,
    payload: &Value,
) -> Result<(), AppServerError> {
    response::begin(
        &fence.mirror_db,
        &scope(fence, owner, &request_value(request))?,
        authority,
        payload,
    )
    .map_err(mutation_held)
}

pub(super) fn finish(
    fence: &RuntimeDeadGenerationFence,
    owner: (&str, u64),
    request: &ServerRequest,
    authority: &Value,
    payload: &Value,
    outcome: &str,
) -> Result<(), AppServerError> {
    response::finish(
        &fence.mirror_db,
        &scope(fence, owner, &request_value(request))?,
        authority,
        payload,
        outcome,
    )
    .map_err(mutation_held)
}
