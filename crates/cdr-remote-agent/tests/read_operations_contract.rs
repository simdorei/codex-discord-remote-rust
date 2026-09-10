use std::fs;
use std::process::Command;

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
async fn read1_rules_and_search_are_bounded_and_hide_sensitive_paths() {
    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join("AGENTS.md"), "Use UTF-8.").expect("rules");
    fs::create_dir(directory.path().join("src")).expect("src");
    fs::write(directory.path().join("src/app.rs"), "let needle = 1;\n").expect("source");
    fs::write(directory.path().join(".env"), "needle=AbCdEfGh12345678\n").expect("secret");
    let dispatcher = bound(directory.path()).await;

    let rules = core(
        dispatcher
            .execute(operation("rules", CoreRequest::ProjectRules), None)
            .await,
    );
    assert!(matches!(
        rules,
        CoreOutput::ProjectRules { ref rules }
            if rules.len() == 1 && rules[0].content == "Use UTF-8."
    ));
    let search = core(
        dispatcher
            .execute(
                operation(
                    "search",
                    CoreRequest::CodeSearch {
                        query: "needle".into(),
                        max_results: 100,
                    },
                ),
                None,
            )
            .await,
    );
    assert!(matches!(
        search,
        CoreOutput::CodeSearch { ref matches }
            if matches.len() == 1 && matches[0].path == "src/app.rs" && matches[0].line == 1
    ));
}

#[tokio::test]
async fn read2_repo_status_diff_and_project_status_match_local_git_state() {
    let directory = tempfile::tempdir().expect("tempdir");
    git(directory.path(), &["init"]);
    git(directory.path(), &["config", "user.name", "Test User"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    fs::write(directory.path().join("notes.txt"), "before\n").expect("initial");
    git(directory.path(), &["add", "notes.txt"]);
    git(directory.path(), &["commit", "-m", "initial"]);
    fs::write(directory.path().join("notes.txt"), "changed\n").expect("change");
    fs::write(directory.path().join("new.txt"), "new line\n").expect("untracked");
    fs::write(
        directory.path().join(".env.local"),
        "TOKEN=AbCdEfGh12345678\n",
    )
    .expect("secret");
    fs::write(directory.path().join("AGENTS.md"), "rules").expect("rules");
    let dispatcher = bound(directory.path()).await;

    let status = core(
        dispatcher
            .execute(operation("status", CoreRequest::RepoStatus), None)
            .await,
    );
    assert!(matches!(
        status,
        CoreOutput::RepoStatus { ref dirty_files, .. }
            if dirty_files.contains(&"notes.txt".to_owned())
                && dirty_files.contains(&"new.txt".to_owned())
                && !dirty_files.iter().any(|path| path.contains(".env"))
    ));
    let diff = core(
        dispatcher
            .execute(operation("diff", CoreRequest::RepoDiff), None)
            .await,
    );
    assert!(matches!(
        diff,
        CoreOutput::RepoDiff { ref files, ref patch, .. }
            if files.iter().any(|file| file.path == "notes.txt")
                && files.iter().any(|file| file.path == "new.txt")
                && patch.contains("+changed") && patch.contains("+new line")
                && !patch.contains("AbCdEfGh12345678") && !patch.contains(".env.local")
    ));
    let project = core(
        dispatcher
            .execute(operation("project", CoreRequest::ProjectStatus), None)
            .await,
    );
    assert!(matches!(
        project,
        CoreOutput::ProjectStatus { ref rule_files, .. } if rule_files == &["AGENTS.md"]
    ));
}

fn git(root: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .expect("git executable");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
