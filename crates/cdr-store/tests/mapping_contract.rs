use std::collections::BTreeSet;

use cdr_store::StoreError;
use cdr_store::mapping::{
    MirrorDetailMode, delete_archived_state, delete_stale, describe_project_channel, find_project,
    get_detail_mode, is_mirrored_channel, mirror_targets, mirrored_thread_id, project_for_channel,
    remaining_discord_ids, set_detail_mode, stale_projects, stale_threads, thread_channels,
    update_discord_thread_id, upsert_project, upsert_thread,
};
use cdr_store::mirror::update_cursor;
use rusqlite::Connection;

#[test]
fn map1_project_alias_merge_is_atomic_and_preserves_thread_mapping() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mapping.sqlite");
    let matcher = |left: &str, right: &str| left.eq_ignore_ascii_case(right);

    let aliases = upsert_project(&path, "Project:Alpha", "Alpha", 10, 100.0, matcher)
        .expect("insert initial project");
    assert!(aliases.is_empty());
    upsert_thread(
        &path,
        "thread-1",
        "Project:Alpha",
        "First thread",
        10,
        101,
        101.0,
    )
    .expect("insert thread under alias");

    let merged = upsert_project(&path, "project:alpha", "Alpha Renamed", 11, 110.0, matcher)
        .expect("merge equivalent project key");
    assert_eq!(merged, vec!["Project:Alpha"]);
    let stored_key: String = Connection::open(&path)
        .expect("open merged mapping")
        .query_row(
            "SELECT project_key FROM mirror_threads WHERE codex_thread_id = 'thread-1'",
            [],
            |row| row.get(0),
        )
        .expect("thread survives alias merge");
    assert_eq!(stored_key, "project:alpha");
    assert_eq!(
        thread_channels(&path, "thread-1").expect("thread channels"),
        Some((10, 101))
    );
    assert_eq!(
        find_project(&path, Some("PROJECT:ALPHA"), matcher)
            .expect("fuzzy project lookup")
            .expect("project match")
            .channel_id,
        11
    );
    assert_eq!(
        project_for_channel(&path, Some(11)).expect("project channel lookup"),
        Some(("project:alpha".into(), "Alpha Renamed".into()))
    );
}

#[test]
fn map2_thread_resolution_detail_and_cleanup_match_python_rules() {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let path = temp.path().join("mapping.sqlite");
    let matcher = |left: &str, right: &str| left == right;
    upsert_project(&path, "project", "Project", 10, 100.0, matcher).expect("insert project");
    upsert_thread(&path, "thread-1", "project", "First", 10, 101, 101.0)
        .expect("insert first thread");

    assert_eq!(
        mirrored_thread_id(&path, Some(101)).expect("exact thread lookup"),
        Some("thread-1".into())
    );
    assert_eq!(
        mirrored_thread_id(&path, Some(10)).expect("single project child lookup"),
        Some("thread-1".into())
    );
    assert_eq!(
        get_detail_mode(&path, "thread-1").expect("default detail"),
        MirrorDetailMode::Send
    );
    set_detail_mode(&path, "thread-1", MirrorDetailMode::All).expect("set all detail");
    assert_eq!(
        get_detail_mode(&path, "thread-1").expect("stored detail"),
        MirrorDetailMode::All
    );
    assert!(matches!(
        set_detail_mode(&path, "missing", MirrorDetailMode::All),
        Err(StoreError::MirrorThreadNotFound(_))
    ));

    upsert_thread(&path, "thread-2", "project", "Second", 10, 102, 102.0)
        .expect("insert second thread");
    assert_eq!(
        mirrored_thread_id(&path, Some(10)).expect("ambiguous project channel"),
        None
    );
    let description = describe_project_channel(&path, Some(10)).expect("describe ambiguity");
    assert!(description.contains("multiple Codex threads"));
    assert!(description.contains("First") && description.contains("Second"));
    assert_eq!(
        update_discord_thread_id(&path, "thread-1", 111, 110.0).expect("update Discord thread id"),
        Some((10, 101))
    );
    assert_eq!(mirror_targets(&path, 10).expect("mirror targets").len(), 2);
    assert!(is_mirrored_channel(&path, Some(111)).expect("updated thread is mirrored"));

    update_cursor(&path, "thread-1", "rollout", 5, 120.0).expect("store archive cursor");
    let remaining = remaining_discord_ids(&path).expect("remaining Discord ids");
    assert_eq!(remaining.thread_ids, BTreeSet::from([102, 111]));
    assert_eq!(remaining.project_channel_ids, vec![10]);
    let deleted = delete_archived_state(&path, "thread-1").expect("delete archived state");
    assert_eq!(deleted.mirror_threads, 1);
    assert_eq!(deleted.session_mirror_offsets, 1);

    let valid_threads = BTreeSet::from(["thread-valid".to_owned()]);
    let valid_projects = BTreeSet::from(["project-valid".to_owned()]);
    assert_eq!(
        stale_threads(&path, &valid_threads, 200.0)
            .expect("stale threads")
            .len(),
        1
    );
    assert_eq!(
        stale_projects(&path, &valid_projects, 200.0)
            .expect("stale projects")
            .len(),
        1
    );
    delete_stale(&path, &valid_threads, &valid_projects, 200.0).expect("delete stale mappings");
    assert!(
        mirror_targets(&path, 10)
            .expect("targets after cleanup")
            .is_empty()
    );
    assert_eq!(
        project_for_channel(&path, Some(10)).expect("project after cleanup"),
        None
    );
}
