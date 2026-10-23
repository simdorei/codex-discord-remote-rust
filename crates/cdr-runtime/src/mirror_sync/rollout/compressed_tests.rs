use super::*;

fn fixture(temp: &tempfile::TempDir) -> (PathBuf, PathBuf, Vec<u8>) {
    let plain = temp.path().join("대화.jsonl");
    let archive = temp.path().join("대화.jsonl.zst");
    let bytes = b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread\"}}\n";
    std::fs::write(&archive, zstd::stream::encode_all(&bytes[..], 1).unwrap()).unwrap();
    (plain, archive, bytes.to_vec())
}

#[test]
fn publication_never_overwrites_a_plain_log_created_during_decompression() {
    let temp = tempfile::tempdir().unwrap();
    let (plain, archive, _) = fixture(&temp);
    let staged = stage(&plain, "thread").unwrap().unwrap();
    let scratch = staged.file.path().to_path_buf();
    std::fs::write(&plain, b"concurrent writer content").unwrap();
    assert_eq!(
        staged.publish(&plain).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read(&plain).unwrap(), b"concurrent writer content");
    assert!(archive.is_file());
    assert!(!scratch.exists());
}

#[cfg(windows)]
#[test]
fn windows_keeps_archive_stable_until_complete_publication() {
    let temp = tempfile::tempdir().unwrap();
    let (plain, archive, bytes) = fixture(&temp);
    let staged = stage(&plain, "thread").unwrap().unwrap();
    assert!(OpenOptions::new().append(true).open(&archive).is_err());
    assert!(std::fs::rename(&archive, temp.path().join("moved.zst")).is_err());
    staged.publish(&plain).unwrap();
    assert_eq!(std::fs::read(&plain).unwrap(), bytes);
    assert!(archive.is_file());
}

#[cfg(unix)]
#[test]
fn replaced_archive_is_reported_without_publishing_a_stale_copy() {
    let temp = tempfile::tempdir().unwrap();
    let (plain, archive, _) = fixture(&temp);
    let staged = stage(&plain, "thread").unwrap().unwrap();
    std::fs::rename(&archive, temp.path().join("old.zst")).unwrap();
    std::fs::write(&archive, b"replacement").unwrap();
    assert!(
        staged
            .publish(&plain)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert!(!plain.exists());
}
