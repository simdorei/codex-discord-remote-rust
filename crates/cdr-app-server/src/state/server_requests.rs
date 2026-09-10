use std::collections::{HashMap, VecDeque};

use crate::{AppServerError, RequestId, ServerRequest, ServerRequestOccurrence};

use super::extract_thread_id;

const MAX_UNSETTLED_SERVER_REQUESTS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServerRequestKey {
    id: RequestId,
    occurrence: ServerRequestOccurrence,
}

impl ServerRequestKey {
    fn new(id: &RequestId, occurrence: ServerRequestOccurrence) -> Self {
        Self {
            id: id.clone(),
            occurrence,
        }
    }

    fn for_request(request: &ServerRequest) -> Self {
        Self::new(&request.id, request.occurrence)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseStatus {
    Responding,
    Indeterminate,
}

#[derive(Debug)]
struct ClaimedServerRequest {
    request: ServerRequest,
    status: ResponseStatus,
}

#[derive(Debug)]
pub(crate) enum ServerRequestRecordError {
    Conflict { id: RequestId },
    Saturated { id: RequestId },
}

#[derive(Debug)]
pub(crate) enum ServerRequestRecordOutcome {
    Broadcast(ServerRequest),
    Duplicate,
    Deferred,
}

#[derive(Debug, Default)]
pub(super) struct ServerRequestState {
    order: VecDeque<ServerRequestKey>,
    pending: HashMap<RequestId, ServerRequest>,
    claimed: HashMap<ServerRequestKey, ClaimedServerRequest>,
    deferred: HashMap<RequestId, ServerRequest>,
}

impl ServerRequestState {
    pub(super) fn record(
        &mut self,
        request: ServerRequest,
    ) -> Result<ServerRequestRecordOutcome, ServerRequestRecordError> {
        let id = request.id.clone();
        if let Some(existing) = self.pending.get(&id) {
            if existing.method == request.method && existing.params == request.params {
                return Ok(ServerRequestRecordOutcome::Duplicate);
            }
            return Err(ServerRequestRecordError::Conflict { id });
        }
        if let Some(claimed) = self
            .claimed
            .values()
            .find(|claimed| claimed.request.id == id)
        {
            // The wire protocol cannot distinguish a retry from identical ID reuse while claimed.
            // Suppress that frame; identical reuse after resolution is recorded as a fresh request.
            if claimed.request.method == request.method && claimed.request.params == request.params
            {
                return Ok(ServerRequestRecordOutcome::Duplicate);
            }
            if let Some(existing) = self.deferred.get(&id) {
                if existing.method == request.method && existing.params == request.params {
                    return Ok(ServerRequestRecordOutcome::Deferred);
                }
                return Err(ServerRequestRecordError::Conflict { id });
            }
            self.ensure_capacity(&id)?;
            self.order
                .push_back(ServerRequestKey::for_request(&request));
            self.deferred.insert(id, request);
            return Ok(ServerRequestRecordOutcome::Deferred);
        }
        self.ensure_capacity(&id)?;
        self.order
            .push_back(ServerRequestKey::for_request(&request));
        self.pending.insert(id, request.clone());
        Ok(ServerRequestRecordOutcome::Broadcast(request))
    }

    fn ensure_capacity(&self, id: &RequestId) -> Result<(), ServerRequestRecordError> {
        if self.order.len() >= MAX_UNSETTLED_SERVER_REQUESTS {
            return Err(ServerRequestRecordError::Saturated { id: id.clone() });
        }
        Ok(())
    }

    pub(super) fn begin_response(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<(), AppServerError> {
        self.response_candidate(id, occurrence)?;
        let key = ServerRequestKey::new(id, occurrence);
        let request = self.pending.remove(id).expect("validated pending request");
        self.claimed.insert(
            key,
            ClaimedServerRequest {
                request,
                status: ResponseStatus::Responding,
            },
        );
        Ok(())
    }

    pub(super) fn response_candidate(
        &self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<&ServerRequest, AppServerError> {
        let key = ServerRequestKey::new(id, occurrence);
        if let Some(claimed) = self.claimed.get(&key) {
            return Err(match claimed.status {
                ResponseStatus::Responding => {
                    AppServerError::ServerRequestResponseInFlight { id: id.clone() }
                }
                ResponseStatus::Indeterminate => {
                    AppServerError::ServerRequestResponseIndeterminate { id: id.clone() }
                }
            });
        }
        let Some(request) = self.pending.get(id) else {
            return Err(AppServerError::StaleServerRequest { id: id.clone() });
        };
        if request.occurrence != occurrence {
            return Err(AppServerError::StaleServerRequest { id: id.clone() });
        }
        Ok(request)
    }

    pub(super) fn mark_indeterminate(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) {
        let key = ServerRequestKey::new(id, occurrence);
        if let Some(claimed) = self.claimed.get_mut(&key)
            && claimed.status == ResponseStatus::Responding
        {
            claimed.status = ResponseStatus::Indeterminate;
        }
    }

    pub(super) fn resolve(
        &mut self,
        id: &RequestId,
        occurrence: ServerRequestOccurrence,
    ) -> Result<Option<ServerRequest>, AppServerError> {
        let key = ServerRequestKey::new(id, occurrence);
        if self.claimed.remove(&key).is_none() {
            return Err(AppServerError::StaleServerRequest { id: id.clone() });
        }
        self.order.retain(|ordered| ordered != &key);
        let promoted = self.deferred.remove(id);
        if let Some(request) = promoted.as_ref() {
            self.pending.insert(id.clone(), request.clone());
        }
        Ok(promoted)
    }

    pub(super) fn pending(&self, thread_id: Option<&str>) -> Vec<ServerRequest> {
        self.order
            .iter()
            .filter_map(|key| {
                self.pending
                    .get(&key.id)
                    .filter(|request| request.occurrence == key.occurrence)
            })
            .filter(|request| matches_thread(request, thread_id))
            .cloned()
            .collect()
    }

    pub(super) fn unsettled(&self, thread_id: Option<&str>) -> Vec<ServerRequest> {
        self.order
            .iter()
            .filter_map(|key| self.request_for_key(key))
            .filter(|request| matches_thread(request, thread_id))
            .cloned()
            .collect()
    }

    pub(super) fn has_unsettled(&self) -> bool {
        !self.pending.is_empty() || !self.claimed.is_empty() || !self.deferred.is_empty()
    }

    pub(super) fn settle_dead_generation_after_exact_match(&mut self) {
        self.order.clear();
        self.pending.clear();
        self.claimed.clear();
        self.deferred.clear();
    }

    fn request_for_key(&self, key: &ServerRequestKey) -> Option<&ServerRequest> {
        self.pending
            .get(&key.id)
            .filter(|request| request.occurrence == key.occurrence)
            .or_else(|| self.claimed.get(key).map(|claimed| &claimed.request))
            .or_else(|| {
                self.deferred
                    .get(&key.id)
                    .filter(|request| request.occurrence == key.occurrence)
            })
    }
}

fn matches_thread(request: &ServerRequest, thread_id: Option<&str>) -> bool {
    thread_id.is_none_or(|target| extract_thread_id(&request.params).as_deref() == Some(target))
}

#[cfg(test)]
#[path = "server_requests_tests.rs"]
mod tests;
