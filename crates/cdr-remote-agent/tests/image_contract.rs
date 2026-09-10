use std::fs;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{CoreOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::{Duration, Utc};

const SESSION: &str = "session-aaaaaaaa";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nremote-image";

async fn bound(root: &std::path::Path) -> LocalProjectDispatcher {
    let dispatcher = LocalProjectDispatcher::new();
    dispatcher
        .upsert("thread-a", root, Utc::now() + Duration::minutes(10))
        .await
        .expect("binding");
    let result = dispatcher
        .execute(
            GatewayCommand::ProjectSession {
                request_id: "activate".into(),
                thread_id: "thread-a".into(),
                deadline_at: Utc::now() + Duration::minutes(1),
                computer_session_id: SESSION.into(),
                computer_session_generation: 1,
            },
            None,
        )
        .await;
    assert!(matches!(result, BridgeResult::ProjectSessionResult { .. }));
    dispatcher
}

fn operation(request_id: &str, request: CoreRequest) -> GatewayCommand {
    GatewayCommand::ProjectOperation {
        request_id: request_id.into(),
        thread_id: "thread-a".into(),
        deadline_at: Utc::now() + Duration::minutes(1),
        computer_session_id: Some(SESSION.into()),
        operation: ProjectOperation::Core(request),
    }
}

fn core(result: BridgeResult) -> CoreOutput {
    match result {
        BridgeResult::ProjectOperationResult {
            output: ProjectOperationOutput::Core(output),
            ..
        } => output,
        unexpected => panic!("unexpected result: {unexpected:?}"),
    }
}

#[tokio::test]
async fn image1_save_list_and_retrieve_validate_exact_image_bytes() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = bound(directory.path()).await;
    let saved = core(
        dispatcher
            .execute(
                operation(
                    "save",
                    CoreRequest::SaveImage {
                        path: "assets/test.png".into(),
                        data_base64: STANDARD.encode(PNG),
                        overwrite: false,
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        saved,
        CoreOutput::ImageSave { ref image, .. }
            if image.path == "assets/test.png" && image.media_type == "image/png"
    ));
    let listed = core(
        dispatcher
            .execute(operation("list", CoreRequest::ListImages), None)
            .await,
    );
    assert!(matches!(
        listed,
        CoreOutput::ImageList { ref images } if images.len() == 1
    ));
    let retrieved = core(
        dispatcher
            .execute(
                operation(
                    "retrieve",
                    CoreRequest::RetrieveImage {
                        path: "assets/test.png".into(),
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        retrieved,
        CoreOutput::ImageRetrieve { ref data_base64, .. }
            if STANDARD.decode(data_base64).expect("base64") == PNG
    ));
    assert_eq!(
        fs::read(directory.path().join("assets/test.png")).unwrap(),
        PNG
    );
}

#[tokio::test]
async fn image2_mismatched_content_and_loopback_url_fail_closed_without_files() {
    let directory = tempfile::tempdir().expect("tempdir");
    let dispatcher = bound(directory.path()).await;
    let mismatched = dispatcher
        .execute(
            operation(
                "mismatch",
                CoreRequest::SaveImage {
                    path: "bad.png".into(),
                    data_base64: STANDARD.encode(b"not a png"),
                    overwrite: false,
                },
            ),
            None,
        )
        .await;
    assert!(matches!(mismatched, BridgeResult::OperationError { .. }));
    let loopback = dispatcher
        .execute(
            operation(
                "loopback",
                CoreRequest::SaveImageFromUrl {
                    path: "from-url.png".into(),
                    url: "https://127.0.0.1/test.png".into(),
                    overwrite: false,
                },
            ),
            None,
        )
        .await;
    assert!(matches!(
        loopback,
        BridgeResult::OperationError { ref message, .. } if message.contains("public network")
    ));
    assert!(!directory.path().join("bad.png").exists());
    assert!(!directory.path().join("from-url.png").exists());
}
