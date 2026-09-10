use crate::{AppServerClient, AppServerError, RequestId, ServerRequestOccurrence};

pub(super) struct ServerResponseClaim {
    client: AppServerClient,
    id: RequestId,
    occurrence: ServerRequestOccurrence,
    resolved: bool,
}

impl ServerResponseClaim {
    pub(super) fn begin_current(
        client: &AppServerClient,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<Self, AppServerError> {
        let mut state = client.inner.state.lock().expect("runtime state lock");
        let expired = || AppServerError::StaleServerRequest { id: id.clone() };
        let request = state.server_response_candidate(id, occurrence)?;
        let thread = crate::extract_thread_id(&request.params).ok_or_else(expired)?;
        let turn = request
            .params
            .get("turnId")
            .and_then(serde_json::Value::as_str)
            .filter(|turn| !turn.is_empty() && turn.trim() == *turn)
            .ok_or_else(expired)?;
        if state.active_turn_id(&thread).as_deref() != Some(turn) {
            return Err(expired());
        }
        state.begin_server_response(id, occurrence)?;
        Ok(Self {
            client: client.clone(),
            id: id.clone(),
            occurrence,
            resolved: false,
        })
    }

    pub(super) fn begin(
        client: &AppServerClient,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<Self, AppServerError> {
        client
            .inner
            .state
            .lock()
            .expect("runtime state lock")
            .begin_server_response(id, occurrence)?;
        Ok(Self {
            client: client.clone(),
            id: id.clone(),
            occurrence,
            resolved: false,
        })
    }

    pub(super) fn resolve(mut self) -> Result<(), AppServerError> {
        let mut state = self.client.inner.state.lock().expect("runtime state lock");
        let promoted = state.resolve_server_request(&self.id, self.occurrence)?;
        if let Some(request) = promoted {
            let _ = self.client.inner.server_requests.send(request);
        }
        drop(state);
        self.resolved = true;
        Ok(())
    }
}

impl Drop for ServerResponseClaim {
    fn drop(&mut self) {
        if !self.resolved {
            self.client
                .inner
                .state
                .lock()
                .expect("runtime state lock")
                .mark_server_response_indeterminate(&self.id, self.occurrence);
        }
    }
}
