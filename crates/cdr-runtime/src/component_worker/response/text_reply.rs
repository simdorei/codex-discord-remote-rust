use cdr_app_server::{
    AppServerError, RequestId, ResidentAppServer, ServerRequest, ServerRequestOccurrence,
    build_approval_response, build_input_response,
};
use serde_json::Value;

use super::is_approval_method;
use crate::component_worker::ComponentWorkerError;

trait PendingTextReplyServer {
    fn generation(&self) -> u64;

    async fn authorize(
        &self,
        request: &ServerRequest,
        generation: u64,
    ) -> Result<(), ComponentWorkerError>;

    async fn pending_server_requests(
        &self,
        thread_id: Option<&str>,
    ) -> Result<Vec<ServerRequest>, AppServerError>;

    async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError>;
}

struct AuthorizedReplyServer<'a> {
    server: &'a ResidentAppServer,
    db: &'a std::path::Path,
    channel: u64,
    user: u64,
}

impl PendingTextReplyServer for AuthorizedReplyServer<'_> {
    fn generation(&self) -> u64 {
        self.server.generation()
    }

    async fn authorize(
        &self,
        request: &ServerRequest,
        generation: u64,
    ) -> Result<(), ComponentWorkerError> {
        crate::server_prompt_authority::verify(self.db, self.server, request, generation)
            .await?
            .require_actor(self.channel, self.user)?;
        Ok(())
    }

    async fn pending_server_requests(
        &self,
        thread_id: Option<&str>,
    ) -> Result<Vec<ServerRequest>, AppServerError> {
        self.server.pending_server_requests(thread_id).await
    }

    async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError> {
        self.server
            .respond_current(id, occurrence, result, expected_generation)
            .await
    }
}

pub async fn handle_pending_text_reply(
    thread_id: &str,
    answer: &str,
    server: &ResidentAppServer,
    db: &std::path::Path,
    channel: u64,
    user: u64,
) -> Result<Option<String>, ComponentWorkerError> {
    handle_pending_text_reply_with(
        thread_id,
        answer,
        &AuthorizedReplyServer {
            server,
            db,
            channel,
            user,
        },
    )
    .await
}

pub(crate) async fn pending_text_reply_available(
    thread_id: &str,
    server: &ResidentAppServer,
) -> Result<bool, ComponentWorkerError> {
    let generation = server.generation();
    let requests = server.pending_server_requests(Some(thread_id)).await?;
    let actual = server.generation();
    if actual != generation {
        return Err(AppServerError::GenerationMismatch {
            expected: generation,
            actual,
        }
        .into());
    }
    Ok(requests.iter().any(|request| {
        request.method == "item/tool/requestUserInput"
            || is_approval_method(&request.method, &request.params)
    }))
}

async fn handle_pending_text_reply_with<S: PendingTextReplyServer>(
    thread_id: &str,
    answer: &str,
    server: &S,
) -> Result<Option<String>, ComponentWorkerError> {
    let generation = server.generation();
    let requests = server.pending_server_requests(Some(thread_id)).await?;
    let actual = server.generation();
    if actual != generation {
        return Err(AppServerError::GenerationMismatch {
            expected: generation,
            actual,
        }
        .into());
    }
    let binding = crate::server_prompt::text_binding::parse(answer)?;
    let mut eligible = requests.iter().filter(|request| {
        (request.method == "item/tool/requestUserInput"
            || is_approval_method(&request.method, &request.params))
            && binding.is_none_or(|(token, _)| {
                crate::server_prompt::text_binding::fingerprint(request, generation) == token
            })
    });
    let Some(request) = eligible.next() else {
        if binding.is_some() {
            return Err(ComponentWorkerError::NoPendingRequest);
        }
        return Ok(None);
    };
    if eligible.next().is_some() {
        return Err(ComponentWorkerError::AmbiguousPendingRequest);
    }
    let answer = binding.map_or(answer, |(_, body)| body);
    server.authorize(request, generation).await?;
    let (payload, confirmation) = if request.method == "item/tool/requestUserInput" {
        (
            build_input_response(&request.params, answer)?.payload,
            "Codex input reply submitted.".to_owned(),
        )
    } else {
        let (payload, action) = build_approval_response(&request.method, &request.params, answer)?;
        (payload, format!("Approval response submitted: {action}"))
    };
    server
        .respond(&request.id, request.occurrence, payload, generation)
        .await?;
    Ok(Some(confirmation))
}

#[cfg(test)]
#[path = "text_reply_tests.rs"]
mod tests;
