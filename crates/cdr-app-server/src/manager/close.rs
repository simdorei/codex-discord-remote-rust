use std::future::Future;

use crate::{AppServerClient, AppServerError};

use super::ResidentAppServer;

pub(super) async fn close(server: &ResidentAppServer) -> Result<(), AppServerError> {
    close_using(server, |client| async move { client.close().await }).await
}

async fn close_using<F, Fut>(
    server: &ResidentAppServer,
    mut cleanup: F,
) -> Result<(), AppServerError>
where
    F: FnMut(AppServerClient) -> Fut,
    Fut: Future<Output = Result<(), AppServerError>>,
{
    let _guard = server.restart_lock.lock().await;
    let plan = server.state.prepare_close();
    server.stop_forwarders().await;
    let mut first_error = None;
    if let Some(replacement) = plan.replacement {
        match cleanup(replacement.client.clone()).await {
            Ok(()) => record_first(
                &mut first_error,
                server.state.finish_replacement_cleanup(&replacement),
            ),
            Err(error) => record_first(&mut first_error, Err(error)),
        }
    }
    if let Some(client) = plan.current {
        let generation = server.generation();
        let cleanup_result = cleanup(client.clone()).await;
        let journal_result = server.settle_exited_idle_owner(&client, generation);
        // Keep this exact sealed client and its wait/reap evidence until both
        // cleanup and the durable exit journal succeed. A failed journal can
        // then be retried by close() without losing proof or launching a child.
        let finished = cleanup_result.is_ok() && journal_result.is_ok();
        record_first(&mut first_error, cleanup_result);
        record_first(&mut first_error, journal_result);
        if finished {
            record_first(&mut first_error, server.state.finish_current_close(&client));
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn record_first(first_error: &mut Option<AppServerError>, result: Result<(), AppServerError>) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

#[cfg(test)]
pub(super) async fn close_with<F, Fut>(
    server: &ResidentAppServer,
    cleanup: F,
) -> Result<(), AppServerError>
where
    F: FnMut(AppServerClient) -> Fut,
    Fut: Future<Output = Result<(), AppServerError>>,
{
    close_using(server, cleanup).await
}
