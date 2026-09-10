use cdr_codex_state::{ContextReadBudget, ContextReadTarget, read_context_batch};

#[test]
fn multi_thread_read_shares_bytes_and_reports_not_scanned_targets() {
    let root = tempfile::tempdir().unwrap();
    let raw = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-a\"}}\n";
    let path = root.path().join("a.jsonl");
    std::fs::write(&path, raw).unwrap();
    let targets = (0..3)
        .map(|_| ContextReadTarget {
            thread: "thread-a".into(),
            path: path.clone(),
        })
        .collect::<Vec<_>>();
    let batch = read_context_batch(
        &targets,
        ContextReadBudget {
            max_bytes: raw.len() as u64,
            ..Default::default()
        },
        2,
    );
    assert_eq!(batch.entries.len(), 2);
    assert_eq!(batch.skipped, 1);
    assert_eq!(batch.entries[0].thread, "thread-a");
    assert!(batch.entries[0].usage.as_ref().unwrap().is_none());
    assert!(
        batch.entries[1]
            .usage
            .as_ref()
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
}

#[test]
fn individual_failure_does_not_hide_other_threads_and_zero_budget_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("b.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-b\"}}\n",
    )
    .unwrap();
    let targets = [
        ContextReadTarget {
            thread: "missing".into(),
            path: root.path().join("missing"),
        },
        ContextReadTarget {
            thread: "thread-b".into(),
            path,
        },
    ];
    let batch = read_context_batch(&targets, ContextReadBudget::default(), 50);
    assert_eq!(batch.entries.len(), 2);
    assert!(batch.entries[0].usage.is_err());
    assert!(batch.entries[1].usage.as_ref().unwrap().is_none());
    let zero = read_context_batch(
        &targets,
        ContextReadBudget {
            max_duration: std::time::Duration::ZERO,
            ..Default::default()
        },
        50,
    );
    assert_eq!(zero.entries.len(), 2);
    assert!(zero.entries.iter().all(|entry| entry.usage.is_err()));
}
