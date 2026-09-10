use std::fs;

use cdr_remote_agent::files::{ProjectFileAccess, RemoteFileError};

#[test]
fn f1_read_is_confined_bounded_hashed_and_redacted() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    fs::write(
        root.join("settings.py"),
        "first\napi_key = \"AbCdEfGh12345678\"\nlast\n",
    )
    .expect("fixture");
    fs::write(directory.path().join("outside.txt"), "private").expect("outside");
    let access = ProjectFileAccess::open(&root).expect("access");

    let output = access.read_file("settings.py", 2, 1).expect("read");
    assert_eq!(output.path, "settings.py");
    assert_eq!(output.content, "api_key = \"[REDACTED]\"");
    assert_eq!(output.start_line, 2);
    assert_eq!(output.end_line, 2);
    assert_eq!(output.total_lines, 3);
    assert!(output.truncated);
    assert!(output.redacted);
    assert_eq!(output.sha256.len(), 64);

    for path in ["../outside.txt", ".env", "private.pem", "service-token.txt"] {
        let error = access.read_file(path, 1, 100).expect_err("unsafe path");
        assert!(matches!(error, RemoteFileError::UnsafePath { .. }));
    }
}

#[test]
fn f2_listing_is_sorted_bounded_and_never_descends_through_links() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    for name in ["c.txt", "a.txt", "b.txt", ".env"] {
        fs::write(root.join(name), name).expect("fixture");
    }
    fs::create_dir(root.join(".git")).expect("git dir");
    fs::write(root.join(".git/secret.txt"), "secret").expect("secret");
    let access = ProjectFileAccess::open(&root).expect("access");

    let output = access.list_files("*.txt", 2).expect("list");
    assert_eq!(
        output
            .files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["a.txt", "b.txt"]
    );
    assert!(output.truncated);
    for pattern in ["../**/*", r"..\**\*", "safe/../outside/*", "/etc/*"] {
        assert!(matches!(
            access.list_files(pattern, 20),
            Err(RemoteFileError::UnsafePattern { .. })
        ));
    }
}

#[test]
fn f3_existing_write_requires_matching_hash_and_publishes_complete_content() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    fs::create_dir(&root).expect("root");
    fs::write(root.join("notes.txt"), "before").expect("fixture");
    let access = ProjectFileAccess::open(&root).expect("access");
    let current = access.read_file("notes.txt", 1, 100).expect("read");

    assert!(matches!(
        access.write_file("notes.txt", "after", None),
        Err(RemoteFileError::Conflict { .. })
    ));
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).expect("unchanged"),
        "before"
    );

    let output = access
        .write_file("notes.txt", "after", Some(&current.sha256))
        .expect("matching write");
    assert!(!output.created);
    assert_eq!(output.bytes_written, 5);
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).expect("updated"),
        "after"
    );

    let created = access
        .write_file("nested/new.txt", "new", None)
        .expect("create");
    assert!(created.created);
    assert_eq!(
        fs::read_to_string(root.join("nested/new.txt")).expect("created"),
        "new"
    );
}

#[test]
fn f4_bound_root_identity_and_single_link_rule_fail_closed() {
    let directory = tempfile::tempdir().expect("tempdir");
    let root = directory.path().join("project");
    let moved = directory.path().join("moved");
    fs::create_dir(&root).expect("root");
    fs::write(root.join("notes.txt"), "safe").expect("fixture");
    let access = ProjectFileAccess::open(&root).expect("access");

    fs::rename(&root, &moved).expect("move bound root");
    fs::create_dir(&root).expect("replacement root");
    assert!(matches!(
        access.verify_root(),
        Err(RemoteFileError::UnsafePath { .. })
    ));

    let root = directory.path().join("links");
    fs::create_dir(&root).expect("links root");
    let outside = directory.path().join("outside-secret");
    fs::write(&outside, "secret").expect("outside");
    fs::hard_link(&outside, root.join("notes.txt")).expect("hard link");
    let access = ProjectFileAccess::open(&root).expect("access");
    assert!(matches!(
        access.read_file("notes.txt", 1, 10),
        Err(RemoteFileError::UnsafePath { .. })
    ));
}
