use super::*;

#[tokio::test]
async fn long_korean_payload_and_attachment_survive_while_nested_credentials_are_display_redacted()
{
    let (_temp, executor, backend) = fixture();
    let prompt = "한글 원문\n<@123> 공백  ".repeat(600);
    let original = json!({
        "prompt":prompt,
        "attachments":[{"filename":"메모.txt","url":"https://example.invalid/메모.txt","details":{"Client-Secret":"fixture-hidden"}}],
        "nested":[{"Proxy_Authorization":"fixture-hidden","OTP-Code":"fixture-hidden","keep":null}],
        "metadata":{"RefreshToken":"fixture-hidden","Cookie":"fixture-hidden","valid":true}
    });
    let before = seed(&executor, "action:large", 99, 20, original);
    let result = executor
        .execute(detail_action("action:large"), 99, 20)
        .await
        .unwrap();
    let displayed = display_payload(&result.text);
    assert_eq!(displayed["prompt"], prompt);
    assert_eq!(displayed["attachments"][0]["filename"], "메모.txt");
    assert_eq!(
        displayed["attachments"][0]["url"],
        "https://example.invalid/메모.txt"
    );
    assert_eq!(displayed["nested"], json!([{"keep":null}]));
    assert_eq!(displayed["metadata"], json!({"valid":true}));
    assert!(!result.text.contains("fixture-hidden"));
    assert_eq!(
        get(executor.mirror_db(), "action:large").unwrap(),
        Some(before)
    );
    assert!(backend.starts.lock().await.is_empty());
    assert!(backend.forks.lock().await.is_empty());
    assert!(backend.resumes.lock().await.is_empty());
}
