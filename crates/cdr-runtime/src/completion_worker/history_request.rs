use std::time::Duration;

use cdr_app_server::requests::{AppRequest, read_thread_with_timeout};

pub(super) fn full_history_request(thread_id: &str, timeout: Duration) -> AppRequest {
    read_thread_with_timeout(thread_id, true, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_history_request_keeps_the_callers_exact_timeout() {
        let timeout = Duration::from_millis(47_123);

        let request = full_history_request("thread-a", timeout);

        assert_eq!(request.method, "thread/read");
        assert_eq!(request.params["threadId"], "thread-a");
        assert_eq!(request.params["includeTurns"], true);
        assert_eq!(request.timeout, timeout);
    }
}
