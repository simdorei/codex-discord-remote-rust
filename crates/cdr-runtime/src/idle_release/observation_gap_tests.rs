//! The existing public gap signal must leave evidence outside process memory.
use cdr_app_server::ResidentAppServer;
use rusqlite::{Connection, params};

#[tokio::test]
async fn patch06_gap_mark_survives_cold_store_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("mirror.sqlite");
    let mut config = crate::soak::native_fixture::config("action");
    config.environment.insert(
        "CDR_ACTION_RPC_LOG".into(),
        temp.path().join("rpc.jsonl").to_string_lossy().into_owned(),
    );
    let server = ResidentAppServer::start(config).await.unwrap();
    super::install(&server, &db).unwrap();
    let owner = server.instance_id().to_owned();
    let generation = i64::try_from(server.generation()).unwrap();

    // Production signal used for broadcast lag and required journal failure.
    server.mark_idle_observation_gap();
    let before_close = unresolved(&db, &owner, generation);
    server.close().await.unwrap();
    drop(server);
    let after_close = unresolved(&db, &owner, generation);

    assert!(
        before_close > 0,
        "a reported gap must have durable unresolved evidence, not only an AtomicBool"
    );
    assert!(
        after_close > 0,
        "a cold reopen or confirmed child exit does not reconstruct missing observations"
    );
}

fn unresolved(path: &std::path::Path, owner: &str, generation: i64) -> i64 {
    let db = Connection::open(path).unwrap();
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema
            WHERE type='table' AND name='cdr_observation_gaps')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if !present {
        // Missing implementation is an assertion failure, not invalid fixture SQL.
        return 0;
    }
    db.query_row(
        "SELECT COUNT(*) FROM cdr_observation_gaps
            WHERE owner_id=?1 AND generation=?2 AND state!='Verified'",
        params![owner, generation],
        |row| row.get(0),
    )
    .unwrap()
}
