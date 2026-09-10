use std::time::Duration;

use cdr_app_server::requests::{AppRequest, resume_thread_with_timeout};

use super::ActionExecutor;
use crate::queue_runner::TurnBackend;

pub(super) fn resume_request(thread_id: &str, timeout: Duration) -> AppRequest {
    resume_thread_with_timeout(thread_id, timeout)
}

impl<B: TurnBackend> ActionExecutor<B> {
    #[must_use]
    pub const fn with_app_server_resume_timeout(mut self, timeout: Duration) -> Self {
        self.app_server_resume_timeout = timeout;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_request_keeps_the_callers_exact_timeout() {
        let timeout = Duration::from_millis(47_123);

        let request = resume_request("thread-a", timeout);

        assert_eq!(request.method, "thread/resume");
        assert_eq!(request.params["threadId"], "thread-a");
        assert_eq!(request.timeout, timeout);
    }
}
