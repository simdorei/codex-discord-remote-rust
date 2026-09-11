#[allow(dead_code)]
#[path = "support/archive_app_server.rs"]
mod app;
#[test]
fn reader_waits_for_the_newline_commit_instead_of_parsing_partial_json() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rpc.jsonl");
    std::fs::write(&path, b"{\"method\":\"initialize\"}\n{\"meth").unwrap();
    let calls = app::calls(&root);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["method"], "initialize");
    std::fs::write(
        &path,
        b"{\"method\":\"initialize\"}\n{\"method\":\"thread/list\"}\n",
    )
    .unwrap();
    assert_eq!(app::calls(&root).len(), 2);
}
#[test]
fn uncommitted_partial_utf8_is_not_a_decode_failure() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("rpc.jsonl"), [b'{', b'"', 0xed, 0x95]).unwrap();
    assert!(app::calls(&root).is_empty());
}
#[test]
#[should_panic(expected = "expected value")]
fn invalid_committed_record_is_not_silently_dropped() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("rpc.jsonl"), b"invalid\n").unwrap();
    let _ = app::calls(&root);
}
