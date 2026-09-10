use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;

use cdr_remote_agent::files::{MAX_FILE_BYTES, ProjectFileAccess, RemoteFileError};

#[test]
fn f5_concurrent_optimistic_writes_have_one_winner_and_no_staging_leaks() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    fs::write(root.join("notes.txt"), "before").expect("fixture");
    let access = Arc::new(ProjectFileAccess::open(&root).expect("access"));
    let current = access.read_file("notes.txt", 1, 10).expect("read");
    let barrier = Arc::new(Barrier::new(3));
    let workers = ["first", "second"].map(|content| {
        let access = Arc::clone(&access);
        let barrier = Arc::clone(&barrier);
        let hash = current.sha256.clone();
        thread::spawn(move || {
            barrier.wait();
            access.write_file("notes.txt", content, Some(&hash))
        })
    });
    barrier.wait();
    let outcomes = workers.map(|worker| worker.join().expect("worker"));
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_err()).count(),
        1
    );
    assert!(matches!(
        fs::read_to_string(root.join("notes.txt"))
            .expect("winner")
            .as_str(),
        "first" | "second"
    ));
    let names = fs::read_dir(&root)
        .expect("entries")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["notes.txt"]);
}

#[test]
fn f6_size_encoding_missing_hash_and_recursive_pattern_fail_closed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    let access = ProjectFileAccess::open(&root).expect("access");
    let oversized = "x".repeat(MAX_FILE_BYTES + 1);
    assert!(matches!(
        access.write_file("large.txt", &oversized, None),
        Err(RemoteFileError::Size { .. })
    ));
    fs::write(root.join("large.txt"), oversized).expect("large fixture");
    assert!(matches!(
        access.read_file("large.txt", 1, 10),
        Err(RemoteFileError::Size { .. })
    ));
    fs::write(root.join("binary.txt"), [0xff, 0xfe]).expect("binary fixture");
    assert!(matches!(
        access.read_file("binary.txt", 1, 10),
        Err(RemoteFileError::Encoding { .. })
    ));
    assert!(matches!(
        access.write_file("missing.txt", "value", Some(&"0".repeat(64))),
        Err(RemoteFileError::Conflict { .. })
    ));
    assert!(matches!(
        access.list_files("**/**/x", 10),
        Err(RemoteFileError::UnsafePattern { .. })
    ));
}

#[test]
fn f7_redaction_matches_the_frozen_python_output() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    let input = concat!(
        "AWS=AKIAABCDEFGHIJKLMNOP\n",
        "aws_secret_access_key=AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCD\n",
        "{\"client_secret\":\"client-value\"}\n",
        "DATABASE_URL=postgres://user:pass@host/db\n",
        "url=https://user:pass@host/x\n",
        "site=https://user@host/x?token=secretvalue123\n",
        "Authorization: Bearer AbCdEfGhIjKlMnOp\n",
        "password=\"supersecret123\"\n",
        "entropy=AbCdEfGhIjKlMnOpQrStUvWxYz012345",
    );
    fs::write(root.join("settings.txt"), input).expect("fixture");
    let output = ProjectFileAccess::open(&root)
        .expect("access")
        .read_file("settings.txt", 1, 100)
        .expect("read");
    assert_eq!(
        output.content,
        concat!(
            "AWS=[REDACTED]\n",
            "aws_secret_access_key=[REDACTED]\n",
            "{\"client_secret\":\"[REDACTED]\"}\n",
            "DATABASE_URL=[REDACTED]\n",
            "url=https://[REDACTED]@host/x\n",
            "site=https://[REDACTED]@host/x?token=[REDACTED]\n",
            "Authorization: Bearer [REDACTED]\n",
            "password=\"[REDACTED]\"\n",
            "entropy=[REDACTED]",
        )
    );
    assert!(output.redacted);
}
