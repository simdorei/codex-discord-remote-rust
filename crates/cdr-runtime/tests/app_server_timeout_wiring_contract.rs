fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect()
}

#[test]
fn tw_1_completion_and_recovery_reads_use_the_configured_history_timeout() {
    let completion = compact(include_str!("../src/completion_worker/context.rs"));
    let recovery = compact(include_str!("../src/completion_worker/recovery.rs"));
    let requests = compact(include_str!("../src/completion_worker/history_request.rs"));
    let runtime = compact(include_str!("../src/discord_runtime.rs"));

    assert!(runtime.contains(
        "run_completion_worker(completion_receiver,Arc::clone(&server),Arc::clone(&queue),Arc::clone(&http),config.stream_commentary,config.app_server_history_read_timeout,completion_shutdown_rx,)"
    ));
    assert!(requests.contains("read_thread_with_timeout(thread_id,true,timeout)"));
    assert_eq!(completion.matches("full_history_request(").count(), 1);
    assert_eq!(recovery.matches("full_history_request(").count(), 2);
    assert_eq!(completion.matches("self.history_read_timeout").count(), 1);
    assert_eq!(recovery.matches("self.history_read_timeout").count(), 2);
}

#[test]
fn tw_2_every_production_action_resume_uses_the_configured_timeout() {
    let executor = compact(include_str!("../src/action_executor.rs"));
    let resume = compact(include_str!("../src/action_executor/resume_action.rs"));
    let archive = compact(include_str!("../src/action_executor/archive_action.rs"));
    let operators = compact(include_str!("../src/action_executor/operator_actions.rs"));
    let requests = compact(include_str!(
        "../src/action_executor/app_server_requests.rs"
    ));
    let bootstrap = compact(include_str!("../src/discord_runtime/bootstrap.rs"));

    assert!(executor.contains("app_server_resume_timeout:Duration"));
    assert!(requests.contains("resume_thread_with_timeout(thread_id,timeout)"));
    assert!(resume.contains("letdeadline=Instant::now()+self.app_server_resume_timeout"));
    assert!(resume.contains("resume_thread_with_timeout(thread,remaining(deadline)?)"));
    assert!(archive.contains("resume_request(thread,self.app_server_resume_timeout)"));
    assert!(operators.contains("resume_request(&target.thread_id,self.app_server_resume_timeout)"));
    assert_eq!(archive.matches("resume_request(").count(), 1);
    assert_eq!(operators.matches("resume_request(").count(), 1);
    assert!(
        bootstrap.contains(".with_app_server_resume_timeout(config.app_server_resume_timeout)")
    );
}

#[test]
fn tw_3_production_sources_have_no_fixed_read_or_resume_request_calls() {
    let sources = [
        include_str!("../src/completion_worker.rs"),
        include_str!("../src/completion_worker/context.rs"),
        include_str!("../src/completion_worker/recovery.rs"),
        include_str!("../src/action_executor/control_actions.rs"),
        include_str!("../src/action_executor/operator_actions.rs"),
        include_str!("../src/action_executor/resume_action.rs"),
    ];

    for source in sources {
        let source = compact(source);
        assert!(
            !source.contains(".execute(read_thread("),
            "legacy fixed 8-second full-history request remains"
        );
        assert!(
            !source.contains(".execute(resume_thread("),
            "legacy fixed 10-second resume request remains"
        );
    }
}
