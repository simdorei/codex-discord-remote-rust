#[path = "support/attachment_http.rs"]
mod http;
use cdr_runtime::admin::attachment::{prepare, target::Target, transport};
use http::Reply;
use std::{fs, path::Path};
fn upload(root: &Path) -> cdr_runtime::admin::attachment::Upload {
    fs::write(root.join("note.txt"), "한글 첨부 내용").unwrap();
    prepare(" hello ", None, &[root.join("note.txt")]).unwrap()
}
#[tokio::test]
async fn real_multipart_preserves_utf8_content_file_precedence_and_exact_receipt() {
    let root = tempfile::tempdir().unwrap();
    let _ = upload(root.path());
    fs::write(root.path().join("caption.txt"), " 파일 내용 \n").unwrap();
    let input = prepare(
        "ignored inline",
        Some(&root.path().join("caption.txt")),
        &[root.path().join("note.txt")],
    )
    .unwrap();
    let (url, received) = http::server(vec![
        Reply::json("200 OK", "{}"),
        Reply::json(
            "200 OK",
            r#"{"id":"987","channel_id":"123","attachments":[{"filename":"note.txt"}]}"#,
        ),
    ])
    .await;
    let receipt = transport::send(
        &url,
        "synthetic-secret",
        &Target::channel("123").unwrap(),
        input,
    )
    .await
    .unwrap();
    assert_eq!(receipt.message_id, "987");
    assert_eq!(receipt.filenames, vec!["note.txt"]);
    let requests = received.await.unwrap();
    assert_eq!(requests.len(), 2);
    let first = String::from_utf8_lossy(&requests[0]);
    let post = String::from_utf8_lossy(&requests[1]);
    assert!(first.starts_with("GET /api/v10/channels/123 "));
    assert!(post.starts_with("POST /api/v10/channels/123/messages "));
    assert!(
        post.to_ascii_lowercase()
            .contains("authorization: bot synthetic-secret")
    );
    assert!(post.contains("name=\"payload_json\""));
    assert!(post.contains("name=\"files[0]\""));
    assert!(post.contains("filename=\"note.txt\""));
    assert!(post.contains("text/plain"));
    assert!(post.contains("파일 내용"));
    assert!(post.contains("한글 첨부 내용"));
    assert!(!post.contains("ignored inline"));
    assert!(post.contains(r#""allowed_mentions":{"parse":[]}"#));
}
#[tokio::test]
async fn inaccessible_channel_never_uploads_and_mirror_has_actionable_diagnosis() {
    for (status, body) in [
        ("403 Forbidden", r#"{"message":"Forbidden"}"#),
        ("404 Not Found", r#"{"message":"Unknown Channel"}"#),
    ] {
        for mirrored in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (url, received) = http::server(vec![Reply::json(status, body)]).await;
            let error = transport::send(
                &url,
                "synthetic-secret",
                &Target {
                    channel_id: "123".into(),
                    mirrored,
                },
                upload(root.path()),
            )
            .await
            .unwrap_err();
            assert!(error.contains(&status[..3]));
            assert!(error.contains(if status.starts_with("403") {
                "Forbidden"
            } else {
                "Unknown Channel"
            }));
            assert_eq!(error.contains("!mirror check, then !mirror sync"), mirrored);
            assert_eq!(received.await.unwrap().len(), 1);
        }
    }
}
#[tokio::test]
async fn upload_failure_is_bounded_redacted_and_not_retried() {
    let root = tempfile::tempdir().unwrap();
    let (url, received) = http::server(vec![
        Reply::json("200 OK", "{}"),
        Reply::json(
            "500 Internal Server Error",
            &format!("synthetic-secret{}", "x".repeat(1200)),
        ),
    ])
    .await;
    let error = transport::send(
        &url,
        "synthetic-secret",
        &Target::channel("123").unwrap(),
        upload(root.path()),
    )
    .await
    .unwrap_err();
    assert!(error.contains("HTTP 500"));
    assert!(error.contains("[REDACTED]"));
    assert!(!error.contains("synthetic-secret"));
    assert!(error.contains("not retried"));
    assert!(error.len() < 1100);
    assert_eq!(received.await.unwrap().len(), 2);
}
#[tokio::test]
async fn uncertain_response_and_wrong_receipt_never_claim_success_or_retry() {
    for body in [
        "not-json",
        r#"{"id":"987","channel_id":"999","attachments":[{"filename":"note.txt"}]}"#,
        r#"{"id":"987","channel_id":"123","attachments":[]}"#,
    ] {
        let root = tempfile::tempdir().unwrap();
        let (url, received) = http::server(vec![
            Reply::json("200 OK", "{}"),
            Reply::json("200 OK", body),
        ])
        .await;
        let error = transport::send(
            &url,
            "synthetic-secret",
            &Target::channel("123").unwrap(),
            upload(root.path()),
        )
        .await
        .unwrap_err();
        assert!(error.contains("outcome unknown, not retried"));
        assert_eq!(received.await.unwrap().len(), 2);
    }
    let root = tempfile::tempdir().unwrap();
    let mut disconnect = Reply::json("200 OK", "");
    disconnect.disconnect = true;
    let (url, received) = http::server(vec![Reply::json("200 OK", "{}"), disconnect]).await;
    let error = transport::send(
        &url,
        "synthetic-secret",
        &Target::channel("123").unwrap(),
        upload(root.path()),
    )
    .await
    .unwrap_err();
    assert!(error.contains("outcome unknown; not retried"));
    assert_eq!(received.await.unwrap().len(), 2);
}
#[tokio::test]
async fn redirects_do_not_forward_the_token_or_upload() {
    let root = tempfile::tempdir().unwrap();
    let mut reply = Reply::json("302 Found", "");
    reply.header = "Location: /redirected\r\n".into();
    let (url, received) = http::server(vec![reply]).await;
    let error = transport::send(
        &url,
        "synthetic-secret",
        &Target::channel("123").unwrap(),
        upload(root.path()),
    )
    .await
    .unwrap_err();
    assert!(error.contains("HTTP 302"));
    assert_eq!(received.await.unwrap().len(), 1);
}
#[test]
fn malformed_utf8_directories_and_oversized_content_are_explicit() {
    let root = tempfile::tempdir().unwrap();
    let _ = upload(root.path());
    let files = [root.path().join("note.txt")];
    fs::write(root.path().join("caption"), [255]).unwrap();
    assert!(
        prepare("", Some(&root.path().join("caption")), &files)
            .err()
            .unwrap()
            .contains("UTF-8")
    );
    assert!(
        prepare(&"😀".repeat(1001), None, &files)
            .err()
            .unwrap()
            .contains("2000")
    );
    assert!(prepare("", None, &[root.path().to_owned()]).is_err());
    assert!(
        prepare("", None, &vec![files[0].clone(); 11])
            .err()
            .unwrap()
            .contains("10")
    );
}
