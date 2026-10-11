//! Cold-start recovery is bounded and completes before queue workers can dispatch.
use cdr_app_server::{ResidentAppServer, requests::AppRequest};
use cdr_store::mutation_attempt::resume_recovery::{self, Candidate};
use serde_json::json;
use std::{path::Path, time::Duration};

pub(super) async fn recover(server: &ResidentAppServer, db: &Path, runtime: &str) {
    let candidates = match resume_recovery::candidates(db, runtime) {
        Ok(candidates) => candidates,
        Err(error) => {
            eprintln!("resume_restart_recovery_scan_failed: {error}");
            return;
        }
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    for candidate in candidates {
        match tokio::time::timeout_at(deadline, observe(server, &candidate)).await {
            Ok(Ok((generation, metadata, latest))) => {
                if server.generation() != generation {
                    break;
                }
                let Ok(generation) = i64::try_from(generation) else {
                    break;
                };
                match resume_recovery::recover(
                    db,
                    runtime,
                    (server.instance_id(), generation),
                    &candidate,
                    &metadata,
                    &latest,
                ) {
                    Ok(jobs) => eprintln!(
                        "resume_restart_recovered: target={} attempt={} preflight_jobs={jobs}",
                        candidate.target, candidate.attempt_id
                    ),
                    Err(error) => eprintln!(
                        "resume_restart_recovery_held: target={} error={error}",
                        candidate.target
                    ),
                }
            }
            Ok(Err(error)) => eprintln!(
                "resume_restart_observation_failed: target={} error={error}",
                candidate.target
            ),
            Err(_) => {
                eprintln!("resume_restart_recovery_budget_exhausted");
                break;
            }
        }
    }
}

async fn observe(
    server: &ResidentAppServer,
    candidate: &Candidate,
) -> Result<(u64, serde_json::Value, serde_json::Value), cdr_app_server::AppServerError> {
    let generation = server.generation();
    let timeout = Duration::from_secs(10);
    let metadata = server
        .execute(
            AppRequest {
                method: "thread/read",
                params: json!({"threadId":candidate.target,"includeTurns":false}),
                timeout,
            },
            Some(generation),
        )
        .await?;
    let latest = server.execute(AppRequest { method:"thread/turns/list",
        params:json!({"threadId":candidate.target,"limit":1,"sortDirection":"desc","itemsView":"notLoaded","cursor":null}),
        timeout }, Some(generation)).await?;
    Ok((generation, metadata, latest))
}

#[cfg(test)]
#[path = "resume_recovery_tests.rs"]
mod tests;
