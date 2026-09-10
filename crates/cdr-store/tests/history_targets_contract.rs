use std::collections::BTreeSet;

use cdr_store::StoreError;
use cdr_store::history::{
    HISTORY_POLL_TARGET_LIMIT, HistoryPollTarget, HistoryTargetSource, history_poll_targets,
};
use cdr_store::mapping::{upsert_project, upsert_thread};

fn target(source: HistoryTargetSource, channel_id: u64) -> HistoryPollTarget {
    HistoryPollTarget { source, channel_id }
}

fn insert_project(path: &std::path::Path, key: &str, channel_id: i64, updated_at: f64) {
    upsert_project(path, key, key, channel_id, updated_at, |left, right| {
        left == right
    })
    .expect("insert mirror project");
}

fn insert_thread(path: &std::path::Path, id: &str, channel_id: i64, updated_at: f64) {
    upsert_thread(path, id, "project", id, 700, channel_id, updated_at)
        .expect("insert mirror thread");
}

#[test]
fn ht1_targets_follow_priority_order_and_keep_the_first_nonzero_occurrence() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("history-targets.sqlite");

    insert_project(&path, "zero-newest", 0, 50.0);
    insert_project(&path, "project-new", 40, 40.0);
    insert_project(&path, "project-next", 45, 30.0);
    insert_project(&path, "allowed-duplicate", 20, 20.0);
    insert_thread(&path, "zero-thread", 0, 60.0);
    insert_thread(&path, "startup-duplicate", 30, 50.0);
    insert_thread(&path, "thread-new", 50, 40.0);
    insert_thread(&path, "project-duplicate", 40, 30.0);
    insert_thread(&path, "thread-next", 60, 20.0);

    let allowed = BTreeSet::from([30, 25, 20, 0]);
    let actual = history_poll_targets(&path, &allowed, Some(30)).expect("select targets");

    assert_eq!(
        actual,
        vec![
            target(HistoryTargetSource::Startup, 30),
            target(HistoryTargetSource::Allowed, 20),
            target(HistoryTargetSource::Allowed, 25),
            target(HistoryTargetSource::MirrorProject, 40),
            target(HistoryTargetSource::MirrorProject, 45),
            target(HistoryTargetSource::MirrorThread, 50),
            target(HistoryTargetSource::MirrorThread, 60),
        ]
    );
    assert_eq!(
        [
            HistoryTargetSource::Startup.label(),
            HistoryTargetSource::Allowed.label(),
            HistoryTargetSource::MirrorProject.label(),
            HistoryTargetSource::MirrorThread.label(),
        ],
        ["startup", "allowed", "mirror_project", "mirror_thread"]
    );
}

#[test]
fn ht2_limit_applies_after_deduplication_across_all_sources() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("history-target-limit.sqlite");
    let allowed = (2_u64..=49).collect::<BTreeSet<_>>();

    for index in 0..60 {
        insert_project(
            &path,
            &format!("allowed-duplicate-{index:02}"),
            20,
            1_000.0 - f64::from(index),
        );
    }
    insert_project(&path, "last-slot", 50, 900.0);
    insert_project(&path, "over-limit", 51, 800.0);
    insert_thread(&path, "also-over-limit", 52, 1_100.0);

    let actual = history_poll_targets(&path, &allowed, Some(1)).expect("select capped targets");

    assert_eq!(actual.len(), HISTORY_POLL_TARGET_LIMIT);
    assert_eq!(
        actual.first(),
        Some(&target(HistoryTargetSource::Startup, 1))
    );
    assert_eq!(
        actual.last(),
        Some(&target(HistoryTargetSource::MirrorProject, 50))
    );
    assert!(!actual.iter().any(|item| item.channel_id == 51));
    assert!(!actual.iter().any(|item| item.channel_id == 52));
}

#[test]
fn ht3_each_call_observes_current_mirror_rows_instead_of_a_cached_list() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("history-target-refresh.sqlite");
    let allowed = BTreeSet::from([7]);

    assert!(
        history_poll_targets(&path, &BTreeSet::new(), Some(0))
            .expect("ignore a zero startup target")
            .is_empty()
    );
    assert_eq!(
        history_poll_targets(&path, &allowed, None).expect("select initial targets"),
        vec![target(HistoryTargetSource::Allowed, 7)]
    );

    insert_project(&path, "old", 80, 10.0);
    insert_project(&path, "new", 90, 20.0);
    insert_thread(&path, "latest-thread", 100, 30.0);

    assert_eq!(
        history_poll_targets(&path, &allowed, None).expect("refresh targets"),
        vec![
            target(HistoryTargetSource::Allowed, 7),
            target(HistoryTargetSource::MirrorProject, 90),
            target(HistoryTargetSource::MirrorProject, 80),
            target(HistoryTargetSource::MirrorThread, 100),
        ]
    );
}

#[test]
fn ht4_equal_timestamps_use_stable_keys_at_the_final_slot_boundary() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("history-target-ties.sqlite");
    let allowed = (1_u64..=47).collect::<BTreeSet<_>>();

    insert_project(&path, "z-project", 60, 100.0);
    insert_project(&path, "a-project", 61, 100.0);
    insert_thread(&path, "z-thread", 70, 200.0);
    insert_thread(&path, "a-thread", 71, 200.0);

    let actual = history_poll_targets(&path, &allowed, None).expect("select tied targets");

    assert_eq!(actual.len(), HISTORY_POLL_TARGET_LIMIT);
    assert_eq!(
        &actual[47..],
        [
            target(HistoryTargetSource::MirrorProject, 61),
            target(HistoryTargetSource::MirrorProject, 60),
            target(HistoryTargetSource::MirrorThread, 71),
        ]
    );
    assert!(!actual.iter().any(|item| item.channel_id == 70));
}

#[test]
fn ht5_startup_and_lowest_allowed_ids_fill_the_cap_before_mirror_targets() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("history-target-config-cap.sqlite");
    let allowed = (1_u64..=100).collect::<BTreeSet<_>>();
    insert_project(&path, "not-admitted", 999, 1_000.0);

    let actual = history_poll_targets(&path, &allowed, Some(50)).expect("select config targets");

    assert_eq!(actual.len(), HISTORY_POLL_TARGET_LIMIT);
    assert_eq!(actual[0], target(HistoryTargetSource::Startup, 50));
    assert_eq!(actual[1], target(HistoryTargetSource::Allowed, 1));
    assert_eq!(actual[49], target(HistoryTargetSource::Allowed, 49));
    assert!(
        actual[1..]
            .iter()
            .all(|item| item.source == HistoryTargetSource::Allowed)
    );
    assert!(!actual.iter().any(|item| item.channel_id == 999));
}

#[test]
fn ht6_negative_persisted_discord_ids_fail_fast_as_database_conversion_errors() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let project_path = temp.path().join("negative-project-id.sqlite");
    insert_project(&project_path, "invalid", -1, 1.0);
    assert_negative_id_error(
        &history_poll_targets(&project_path, &BTreeSet::new(), None)
            .expect_err("negative project channel must fail"),
    );

    let thread_path = temp.path().join("negative-thread-id.sqlite");
    insert_thread(&thread_path, "invalid", -2, 1.0);
    assert_negative_id_error(
        &history_poll_targets(&thread_path, &BTreeSet::new(), None)
            .expect_err("negative thread channel must fail"),
    );
}

fn assert_negative_id_error(error: &StoreError) {
    assert!(matches!(
        error,
        StoreError::Database(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            _
        ))
    ));
}
