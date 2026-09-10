use std::fs;
use std::process::{Command, Output};

use cdr_remote_agent::dispatcher::LocalProjectDispatcher;
use cdr_remote_protocol::message::{BridgeResult, GatewayCommand};
use cdr_remote_protocol::output::{CoreOutput, ProjectOperationOutput};
use cdr_remote_protocol::request::{CoreRequest, ProjectOperation};
use chrono::{Duration, Utc};

const SESSION: &str = "session-aaaaaaaa";

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
        deadline_at: Utc::now() + Duration::minutes(5),
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
async fn git1_commit_selected_path_preserves_an_unrelated_staged_file() {
    let directory = tempfile::tempdir().expect("tempdir");
    initialize(directory.path());
    fs::write(directory.path().join("selected.txt"), "selected\n").expect("selected");
    fs::write(directory.path().join("existing.txt"), "existing\n").expect("existing");
    git_ok(directory.path(), &["add", "existing.txt"]);
    let dispatcher = bound(directory.path()).await;

    let result = core(
        dispatcher
            .execute(
                operation(
                    "commit",
                    CoreRequest::GitCommit {
                        message: "test: selected only".into(),
                        paths: vec!["selected.txt".into()],
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        result,
        CoreOutput::GitCommit { ref commit, ref staged_files, .. }
            if commit.len() >= 7 && staged_files == &["selected.txt"]
    ));
    assert_eq!(
        git_text(directory.path(), &["diff", "--cached", "--name-only"]),
        "existing.txt"
    );
}

#[tokio::test]
async fn git2_push_sends_only_the_selected_branch_to_a_configured_local_remote() {
    let directory = tempfile::tempdir().expect("tempdir");
    let project = directory.path().join("project");
    let remote = directory.path().join("remote.git");
    fs::create_dir(&project).expect("project");
    fs::create_dir(&remote).expect("remote");
    initialize(&project);
    git_ok(&remote, &["init", "--bare"]);
    git_ok(
        &project,
        &["remote", "add", "origin", &remote.display().to_string()],
    );
    let branch = git_text(&project, &["branch", "--show-current"]);
    let dispatcher = bound(&project).await;

    let result = core(
        dispatcher
            .execute(
                operation(
                    "push",
                    CoreRequest::GitPush {
                        remote: "origin".into(),
                        branch: Some(branch.clone()),
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        result,
        CoreOutput::GitPush { ref branch, .. } if branch == &git_text(&project, &["branch", "--show-current"])
    ));
    assert!(!git_text(&remote, &["rev-parse", &format!("refs/heads/{branch}")]).is_empty());
}

fn initialize(root: &std::path::Path) {
    git_ok(root, &["init"]);
    git_ok(root, &["config", "user.name", "Test User"]);
    git_ok(root, &["config", "user.email", "test@example.com"]);
    fs::write(root.join("notes.txt"), "before\n").expect("initial");
    git_ok(root, &["add", "notes.txt"]);
    git_ok(root, &["commit", "-m", "initial"]);
}

fn git(root: &std::path::Path, arguments: &[&str]) -> Output {
    Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("git executable")
}

fn git_ok(root: &std::path::Path, arguments: &[&str]) {
    let output = git(root, arguments);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(root: &std::path::Path, arguments: &[&str]) -> String {
    let output = git(root, arguments);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
