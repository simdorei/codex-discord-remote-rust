use cdr_runtime::prefix_plan::{
    MirrorDetailMode, PrefixAction, PrefixPlanError, SkillPromptKind, plan_prefix,
};

#[test]
fn forced_restart_is_explicit_and_matches_the_emergency_gateway_route() {
    assert_eq!(plan_prefix("restart_codex"), Ok(PrefixAction::RestartCodex));
    for command in [
        "restart_codex force",
        "restart_codex --force",
        "force_restart",
        "RESTART_CODEX FORCE",
    ] {
        assert_eq!(plan_prefix(command), Ok(PrefixAction::ForceRestartCodex));
        assert!(cdr_discord::gateway::ingress::is_force_restart_message(
            &format!("!{command}")
        ));
    }
    for command in [
        "restart_codex maybe",
        "restart_codex force later",
        "force_restart now",
    ] {
        assert!(plan_prefix(command).is_err());
        assert!(!cdr_discord::gateway::ingress::is_force_restart_message(
            &format!("!{command}")
        ));
    }
}

#[test]
fn help_bridge_and_thread_commands_preserve_python_aliases_and_limits() {
    assert_eq!(plan_prefix(""), Ok(PrefixAction::Help));
    assert_eq!(plan_prefix("START"), Ok(PrefixAction::Help));
    assert_eq!(plan_prefix("list"), Ok(PrefixAction::List { limit: 0 }));
    assert_eq!(
        plan_prefix("list 999"),
        Ok(PrefixAction::List { limit: 30 })
    );
    assert_eq!(
        plan_prefix("archive_list nonsense"),
        Ok(PrefixAction::ArchivedList { limit: 10 })
    );
    assert_eq!(
        plan_prefix("open_abort abc"),
        Ok(PrefixAction::Open {
            reference: "abc".into(),
            abort: true,
        })
    );
    assert_eq!(
        plan_prefix("status thread-1"),
        Ok(PrefixAction::Status {
            reference: Some("thread-1".into()),
        })
    );
}

#[test]
fn settings_parser_keeps_python_option_aliases_and_option_discovery() {
    assert_eq!(
        plan_prefix("setting"),
        Ok(PrefixAction::Settings {
            reference: None,
            model: None,
            effort: None,
            speed: None
        })
    );
    assert_eq!(
        plan_prefix("settings --model"),
        Ok(PrefixAction::SettingsOptions {
            reference: None,
            field: Some("model".into()),
        })
    );
    assert_eq!(
        plan_prefix("settings abc --model gpt-5.6-sol --effort ultra --speed fast"),
        Ok(PrefixAction::Settings {
            reference: Some("abc".into()),
            model: Some("gpt-5.6-sol".into()),
            effort: Some("ultra".into()),
            speed: Some("fast".into()),
        })
    );
    assert!(matches!(
        plan_prefix("settings --mystery x"),
        Err(PrefixPlanError::Usage(_))
    ));
}

#[test]
fn status_mirror_queue_and_interactive_aliases_are_complete() {
    assert_eq!(plan_prefix("whoami"), Ok(PrefixAction::Identity));
    assert_eq!(plan_prefix("map"), Ok(PrefixAction::Where));
    assert_eq!(
        plan_prefix("ctx refresh 99"),
        Ok(PrefixAction::Context {
            all_threads: false,
            refresh: true,
            limit: 30,
        })
    );
    assert_eq!(
        plan_prefix("quota nope"),
        Err(PrefixPlanError::Usage("Usage: !usage [days]".into()))
    );
    assert_eq!(plan_prefix("queues"), Ok(PrefixAction::Runners));
    assert_eq!(plan_prefix("system"), Ok(PrefixAction::Resources));
    assert_eq!(
        plan_prefix("unqueue job-1"),
        Ok(PrefixAction::Retract {
            reference: Some("job-1".into()),
        })
    );
    assert_eq!(plan_prefix("approve"), Ok(PrefixAction::Approval));
    assert_eq!(
        plan_prefix("resume"),
        Ok(PrefixAction::Resume { reference: None })
    );
}

#[test]
fn bridge_mirror_and_detail_grammars_match_python() {
    assert_eq!(
        plan_prefix("bridge sync 999"),
        Ok(PrefixAction::BridgeSync { limit: Some(100) })
    );
    assert_eq!(
        plan_prefix("resync"),
        Ok(PrefixAction::BridgeSync { limit: None })
    );
    assert_eq!(plan_prefix("mirror"), Ok(PrefixAction::MirrorSync));
    assert_eq!(
        plan_prefix("mirror list 7"),
        Ok(PrefixAction::MirrorList { limit: Some(7) })
    );
    assert_eq!(
        plan_prefix("mirror doctor"),
        Ok(PrefixAction::MirrorCheck { limit: None })
    );
    assert_eq!(
        plan_prefix("detail send"),
        Ok(PrefixAction::MirrorDetail {
            mode: Some(MirrorDetailMode::Send),
        })
    );
    assert!(matches!(
        plan_prefix("detail verbose"),
        Err(PrefixPlanError::Usage(_))
    ));
}

#[test]
fn prompt_admin_and_qa_commands_keep_required_confirmation_contracts() {
    assert_eq!(
        plan_prefix("pro inspect this"),
        Ok(PrefixAction::SkillPrompt {
            kind: SkillPromptKind::Pro,
            request: "inspect this".into(),
        })
    );
    assert_eq!(
        plan_prefix("deep-interview migrate it"),
        Ok(PrefixAction::SkillPrompt {
            kind: SkillPromptKind::Interview,
            request: "migrate it".into(),
        })
    );
    assert_eq!(
        plan_prefix("archive-used 123M"),
        Ok(PrefixAction::SkillPrompt {
            kind: SkillPromptKind::ArchiveUsed,
            request: "123M".into(),
        })
    );
    assert_eq!(plan_prefix("qa"), Ok(PrefixAction::QaButtons));
    assert_eq!(
        plan_prefix("reset_pc confirm"),
        Ok(PrefixAction::HostReboot)
    );
    assert!(matches!(
        plan_prefix("reset_pc now"),
        Err(PrefixPlanError::Usage(_))
    ));
    assert!(matches!(
        plan_prefix("delete_archive"),
        Err(PrefixPlanError::Usage(_))
    ));
    assert_eq!(
        plan_prefix("delete_archive abc"),
        Ok(PrefixAction::DeleteArchivePreview {
            reference: "abc".into(),
        })
    );
    assert_eq!(
        plan_prefix("confirm_delete_archive abc"),
        Ok(PrefixAction::DeleteArchiveConfirm {
            reference: "abc".into(),
        })
    );
}

#[test]
fn unknown_and_missing_required_arguments_are_explicit_errors() {
    assert_eq!(
        plan_prefix("new"),
        Ok(PrefixAction::New {
            prompt: String::new()
        })
    );
    assert!(matches!(
        plan_prefix("steer"),
        Err(PrefixPlanError::Usage(_))
    ));
    assert_eq!(
        plan_prefix("does_not_exist"),
        Err(PrefixPlanError::Unknown("does_not_exist".into()))
    );
}
