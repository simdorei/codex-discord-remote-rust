use cdr_runtime::admin::{
    env_file,
    setup::{self, SetupInput},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        while !request.windows(4).any(|slice| slice == b"\r\n\r\n") {
            let count = stream.read(&mut buffer).await.unwrap();
            assert_ne!(count, 0);
            request.extend_from_slice(&buffer[..count]);
        }
        stream.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    (format!("http://{address}/applications/@me"), task)
}

#[tokio::test]
async fn setup_checks_token_then_atomically_merges_channel_without_disclosing_token() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".env");
    std::fs::write(
        &path,
        "# 한글\nDISCORD_ALLOWED_CHANNEL_IDS=111\nDISCORD_STARTUP_CHANNEL_ID=555\nOTHER=keep\n",
    )
    .unwrap();
    let (url, received) = server("200 OK", r#"{"id":"42","name":"테스트 봇"}"#).await;
    let output = setup::configure(
        root.path(),
        SetupInput {
            token: "fake-secret=tail".into(),
            channel_id: "222".into(),
        },
        &reqwest::Client::new(),
        &url,
    )
    .await
    .unwrap();
    let request = received.await.unwrap();
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bot fake-secret=tail")
    );
    assert!(!output.contains("fake-secret"));
    assert!(output.contains("테스트 봇"));
    let text = std::fs::read_to_string(path).unwrap();
    assert_eq!(
        env_file::get(&text, "DISCORD_BOT_TOKEN"),
        Some("fake-secret=tail")
    );
    assert_eq!(
        env_file::get(&text, "DISCORD_ALLOWED_CHANNEL_IDS"),
        Some("111,222")
    );
    assert_eq!(
        env_file::get(&text, "DISCORD_STARTUP_CHANNEL_ID"),
        Some("555")
    );
    assert!(text.contains("OTHER=keep"));
}

#[tokio::test]
async fn rejected_token_or_malformed_response_preserves_all_existing_configuration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".env");
    std::fs::write(&path, "UNCHANGED=yes\n").unwrap();
    for (status, body, expected) in [
        ("401 Unauthorized", "fake-secret", "HTTP 401"),
        ("200 OK", "not JSON", "not valid JSON"),
        ("200 OK", r#"{"id":""}"#, "digits only"),
    ] {
        let (url, received) = server(status, body).await;
        let error = setup::configure(
            root.path(),
            SetupInput {
                token: "fake-secret".into(),
                channel_id: String::new(),
            },
            &reqwest::Client::new(),
            &url,
        )
        .await
        .unwrap_err();
        received.await.unwrap();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("fake-secret"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "UNCHANGED=yes\n");
    }
}

#[tokio::test]
async fn invalid_channel_is_rejected_before_network_or_token_save() {
    let root = tempfile::tempdir().unwrap();
    for id in ["general", "0", "18446744073709551616", "１２３"] {
        let error = setup::configure(
            root.path(),
            SetupInput {
                token: "fake-secret".into(),
                channel_id: id.into(),
            },
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("Discord ID"));
        assert!(!root.path().join(".env").exists());
    }
}

#[test]
fn env_update_replaces_all_duplicate_keys_and_preserves_bom_encoded_korean() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".env");
    std::fs::write(&path, "\u{feff}# 유지\r\nVALUE=one\r\nVALUE=two\r\n").unwrap();
    env_file::update(&path, &[("VALUE", "new=value")]).unwrap();
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "# 유지\r\nVALUE=new=value\r\nVALUE=new=value\r\n"
    );
}

#[cfg(windows)]
#[test]
fn env_update_preserves_existing_windows_access_rules() {
    fn access_rules(path: &std::path::Path, protect: bool) -> String {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                "$ErrorActionPreference='Stop'; $acl=[IO.File]::GetAccessControl($env:CDR_TEST_ACL_PATH); if ($env:CDR_TEST_ACL_PROTECT -eq 'true') { $acl.SetAccessRuleProtection($true,$true); [IO.File]::SetAccessControl($env:CDR_TEST_ACL_PATH,$acl) }; [IO.File]::GetAccessControl($env:CDR_TEST_ACL_PATH).GetSecurityDescriptorSddlForm([Security.AccessControl.AccessControlSections]::Access)",
            ])
            .env("CDR_TEST_ACL_PATH", path)
            .env("CDR_TEST_ACL_PROTECT", protect.to_string())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".env");
    std::fs::write(&path, "EXISTING=keep\n").unwrap();
    let before = access_rules(&path, true);
    assert!(
        before.contains("D:P"),
        "test file must have a protected ACL: {before}"
    );
    env_file::update(&path, &[("VALUE", "new")]).unwrap();
    assert_eq!(access_rules(&path, false), before);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "EXISTING=keep\nVALUE=new\n"
    );
}
