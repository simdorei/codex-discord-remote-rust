use cdr_app_server::{AppServerError, RequestId, ResidentAppServer, ServerRequestOccurrence};
use serde_json::Value;

use super::ComponentResponse;

trait ComponentResponseServer {
    async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError>;
}

impl ComponentResponseServer for ResidentAppServer {
    async fn respond(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
        result: Value,
        expected_generation: u64,
    ) -> Result<(), AppServerError> {
        ResidentAppServer::respond_current(self, id, occurrence, result, expected_generation).await
    }
}

pub(in crate::component_worker) async fn submit_component_response(
    response: ComponentResponse,
    server: &ResidentAppServer,
) -> Result<(), AppServerError> {
    submit_component_response_with(response, server).await
}

async fn submit_component_response_with<S: ComponentResponseServer>(
    response: ComponentResponse,
    server: &S,
) -> Result<(), AppServerError> {
    server
        .respond(
            &response.request_id,
            response.occurrence,
            response.payload,
            response.generation,
        )
        .await
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cdr_app_server::{AppServerError, RequestId, ServerRequestOccurrence};
    use serde_json::{Value, json};

    use super::{ComponentResponseServer, submit_component_response_with};
    use crate::component_worker::ComponentResponse;

    struct FakeServer {
        accepted: RefCell<Vec<(RequestId, ServerRequestOccurrence, Value, u64)>>,
    }

    impl ComponentResponseServer for FakeServer {
        async fn respond(
            &self,
            id: &RequestId,
            occurrence: ServerRequestOccurrence,
            result: Value,
            expected_generation: u64,
        ) -> Result<(), AppServerError> {
            self.accepted
                .borrow_mut()
                .push((id.clone(), occurrence, result, expected_generation));
            Ok(())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn submission_passes_the_exact_request_occurrence_generation_triple() {
        let occurrence = ServerRequestOccurrence::from_bytes([0x11; 16]);
        let response = ComponentResponse {
            request_id: RequestId::String("request-7".into()),
            occurrence,
            payload: json!({"decision":"accept"}),
            confirmation: "submitted".into(),
            generation: 3,
        };
        let server = FakeServer {
            accepted: RefCell::new(Vec::new()),
        };

        submit_component_response_with(response, &server)
            .await
            .unwrap();

        assert_eq!(
            *server.accepted.borrow(),
            vec![(
                RequestId::String("request-7".into()),
                occurrence,
                json!({"decision":"accept"}),
                3,
            )]
        );
    }
}
