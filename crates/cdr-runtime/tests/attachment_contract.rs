use cdr_runtime::attachments::{is_text_attachment, render_attachment_prompt, sanitize_filename};

#[test]
fn filenames_are_leaf_only_bounded_and_cross_platform_safe() {
    assert_eq!(sanitize_filename("../../bad:name?.txt", 1), "bad_name_.txt");
    assert_eq!(sanitize_filename("...", 2), "attachment-2");
    assert!(sanitize_filename(&"a".repeat(200), 3).len() <= 120);
}

#[test]
fn text_detection_matches_python_extension_and_content_type_contract() {
    assert!(is_text_attachment("notes.rs", None));
    assert!(is_text_attachment("blob.bin", Some("text/plain")));
    assert!(!is_text_attachment("photo.png", Some("image/png")));
}

#[test]
fn attachment_prompt_exposes_saved_paths_failures_and_bounded_previews() {
    let prompt = render_attachment_prompt(
        "Inspect these",
        &[
            "1. notes.txt\n   path: C:/safe/notes.txt".into(),
            "2. huge.bin skipped: limit".into(),
        ],
        &[("notes.txt".into(), "hello".into())],
    );
    assert!(prompt.contains("Discord attachments saved locally:"));
    assert!(prompt.contains("C:/safe/notes.txt"));
    assert!(prompt.contains("huge.bin skipped"));
    assert!(prompt.contains("--- notes.txt ---\n```text\nhello\n```"));
}
