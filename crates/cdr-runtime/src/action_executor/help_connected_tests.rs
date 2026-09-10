use crate::test_support::{approval_http, mapped_slash as slash, message_fixture::MessageFixture};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn unsupported_detail_reports_real_error_without_mirror_action() {
    let root = tempfile::tempdir().unwrap();
    let http = approval_http::start().await;
    let client = Arc::new(
        twilight_http::Client::builder()
            .proxy(http.address, true)
            .ratelimiter(None)
            .build(),
    );
    let fixture = MessageFixture::new(&root, client.clone()).await;
    let context = fixture.context(root.path());
    let api = cdr_discord::http::DiscordHttp::new(client, twilight_model::id::Id::new(1));
    let target = crate::message_worker::ErrorReportTarget {
        channel_id: twilight_model::id::Id::new(42),
        message_id: twilight_model::id::Id::new(801),
    };
    crate::message_worker::process_with_error_report(
        target,
        fixture.admit("!detail all"),
        |work| crate::message_worker::process_admitted_gateway_message(work, &context),
        |target, error| {
            crate::message_worker::report_processing_error(
                fixture.executor.mirror_db(),
                &api,
                target,
                error,
            )
        },
    )
    .await
    .unwrap();
    http.stop.send(()).unwrap();
    let traffic = http.task.await.unwrap();
    assert_eq!(traffic.len(), 1);
    let text = traffic[0].1["content"].as_str().unwrap();
    assert!(text.contains("ERROR: prefix command is parsed but not implemented yet: !detail"));
    assert!(!text.contains("Mirror sync complete"));
    assert!(!root.path().join("bridge.json").exists());
    fixture.server.close().await.unwrap();
    let frames = crate::test_support::app_fixture::rpc_log(&root.path().join("rpc.jsonl"));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["method"], "initialize");
}

#[tokio::test]
async fn help_reaches_prefix_and_original_slash_with_options_and_surface_limits() {
    for is_slash in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let http = approval_http::start().await;
        let client = Arc::new(
            twilight_http::Client::builder()
                .proxy(http.address, true)
                .ratelimiter(None)
                .build(),
        );
        let fixture = MessageFixture::new(&root, client.clone()).await;
        let direct = fixture
            .executor
            .execute(crate::command_plan::CommandAction::Help, 42, 3)
            .await
            .unwrap();
        assert_eq!(direct.text, super::HELP);
        let server = fixture.server.clone();
        if is_slash {
            let work = slash::stage(fixture.executor.mirror_db(), "help").await;
            let (send, recv) = tokio::sync::mpsc::channel(1);
            send.send(work).await.unwrap();
            drop(send);
            tokio::time::timeout(
                Duration::from_secs(8),
                crate::interaction_worker::run_interaction_worker(
                    recv,
                    Arc::new(fixture.executor),
                    server.clone(),
                    client,
                ),
            )
            .await
            .unwrap();
        } else {
            crate::message_worker::process_admitted_gateway_message(
                fixture.admit("!help"),
                &fixture.context(root.path()),
            )
            .await
            .unwrap();
        }
        http.stop.send(()).unwrap();
        let traffic = http.task.await.unwrap();
        assert_eq!(traffic[0].0, !is_slash);
        let text = traffic
            .iter()
            .map(|(_, value)| value["content"].as_str().unwrap())
            .collect::<String>();
        for required in [
            "!settings [ref] --speed",
            "!context refresh",
            "!usage [days]",
            "!mirror list",
            "! 명령 전용",
            "별칭",
            "!archive",
            "!new <요청>",
            "구현·검증 미완료",
            "Windows 주소 연결 등록 진단",
        ] {
            assert!(text.contains(required), "missing {required}: {text}");
        }
        assert!(!text.contains("ERROR:"));
        assert!(
            !text
                .lines()
                .any(|line| line.starts_with("!ask") || line.starts_with("!use "))
        );
        assert!(!root.path().join("bridge.json").exists());
        server.close().await.unwrap();
        let frames = crate::test_support::app_fixture::rpc_log(&root.path().join("rpc.jsonl"));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["method"], "initialize");
    }
}
