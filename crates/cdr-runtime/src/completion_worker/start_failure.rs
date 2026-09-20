//! Deliver a genuine no-turn start rejection through its own guarded receipt domain.
use super::{CompletionWorkerError, IdempotentChunk, i64_channel};
use cdr_store::reserve_policy::start_notice;

pub(crate) async fn deliver_start_failures(
    db: &std::path::Path,
    http: &twilight_http::Client,
) -> Result<(), CompletionWorkerError> {
    let mut first_error = None;
    for notice in start_notice::pending(db)? {
        let result = async {
            // This is an error notice, not generated output or an accepted first
            // turn. Atomic receipt authorization validates exact no-turn custody
            // without opening the /new first-answer barrier.
            super::receipt::send_chunk(
                db,
                http,
                i64_channel(notice.channel_id)?,
                &IdempotentChunk {
                    domain: start_notice::DOMAIN,
                    logical_key: notice.job_id.clone(),
                    chunk_index: 0,
                    content: notice.content.clone(),
                },
            )
            .await?;
            start_notice::complete(db, &notice.job_id)?;
            Ok::<(), CompletionWorkerError>(())
        }
        .await;
        if let Err(error) = result {
            eprintln!(
                "start_failure_notice_deferred job_id={} error={error}",
                notice.job_id
            );
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}
