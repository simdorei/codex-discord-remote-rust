use super::{
    AppServerError, DeadGenerationFence, DeadGenerationWork, PathBuf, RequestId,
    RuntimeDeadGenerationFence, ServerRequest, Value,
};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct StopAtWriter {
    inner: RuntimeDeadGenerationFence,
    db: PathBuf,
    checks: AtomicUsize,
}

impl StopAtWriter {
    pub fn new(inner: RuntimeDeadGenerationFence, db: PathBuf) -> Self {
        Self {
            inner,
            db,
            checks: AtomicUsize::new(0),
        }
    }
}

impl DeadGenerationFence for StopAtWriter {
    fn persist(&self, work: &DeadGenerationWork) -> Result<(), AppServerError> {
        self.inner.persist(work)
    }

    fn check_request(
        &self,
        generation: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), AppServerError> {
        if method == "server/response"
            && params["threadId"] == "thread-b"
            && self.checks.fetch_add(1, Ordering::AcqRel) == 1
        {
            // The second callback is inside the actual stdin writer lock, after capture.
            cdr_store::schema::open_initialized(&self.db).unwrap().execute(
                "INSERT INTO cdr_execution_holds VALUES('original-a','thread-b','writer stop','{}',1)",[])
                .unwrap();
        }
        self.inner.check_request(generation, method, params)
    }

    fn begin_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        method: &str,
        params: &Value,
        scoped: bool,
    ) -> Result<bool, AppServerError> {
        self.inner
            .begin_mutation(owner, request, method, params, scoped)
    }

    fn finish_mutation(
        &self,
        owner: (&str, u64),
        request: (&str, &RequestId),
        outcome: &str,
    ) -> Result<(), AppServerError> {
        self.inner.finish_mutation(owner, request, outcome)
    }

    fn response_authority(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
    ) -> Result<Option<Value>, AppServerError> {
        self.inner.response_authority(owner, request)
    }

    fn begin_response(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
        authority: &Value,
        payload: &Value,
    ) -> Result<(), AppServerError> {
        self.inner
            .begin_response(owner, request, authority, payload)
    }

    fn finish_response(
        &self,
        owner: (&str, u64),
        request: &ServerRequest,
        authority: &Value,
        payload: &Value,
        outcome: &str,
    ) -> Result<(), AppServerError> {
        self.inner
            .finish_response(owner, request, authority, payload, outcome)
    }
}
