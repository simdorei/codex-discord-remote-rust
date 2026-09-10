use std::path::{Path, PathBuf};

use cdr_pro::prompt::{
    DeviceTicket, PRO_SKILL_CALL, build_turn_input, format_local_device_prompt, is_pro_command,
    is_pro_skill_prompt, pro_conversation_scope, rewrite_pro_prompt,
};
use serde_json::json;

#[test]
fn pro_prompt_attaches_exact_skill_and_chrome_mention_inputs() {
    let skill_path = Path::new("C:/plugin/skills/ask-chatgpt-pro/SKILL.md");
    let prompt = format!("{PRO_SKILL_CALL} review this patch");
    assert!(is_pro_skill_prompt(&prompt));
    assert_eq!(
        build_turn_input(&prompt, skill_path),
        vec![
            json!({"type": "text", "text": prompt, "text_elements": []}),
            json!({
                "type": "skill",
                "name": "ask-chatgpt-pro",
                "path": "C:/plugin/skills/ask-chatgpt-pro/SKILL.md"
            }),
            json!({
                "type": "mention",
                "name": "Chrome",
                "path": "plugin://chrome@openai-bundled"
            }),
        ]
    );
}

#[test]
fn similar_prefix_does_not_gain_pro_capabilities() {
    let skill_path = Path::new("skill.md");
    for prompt in [
        "$ask-chatgpt-probe",
        "$ask-chatgpt-prox",
        "x$ask-chatgpt-pro",
    ] {
        assert!(!is_pro_skill_prompt(prompt));
        assert_eq!(
            build_turn_input(prompt, skill_path),
            vec![json!({"type": "text", "text": prompt, "text_elements": []})]
        );
    }
}

#[test]
fn bang_pro_rewrite_matches_command_boundary_and_review_contract() {
    assert_eq!(
        rewrite_pro_prompt("  !PRO   review   abc  "),
        Some(format!("{PRO_SKILL_CALL} <pro-review>\nabc  "))
    );
    assert_eq!(
        rewrite_pro_prompt("!pro inspect"),
        Some(format!("{PRO_SKILL_CALL} inspect"))
    );
    assert_eq!(rewrite_pro_prompt("!pro"), Some(PRO_SKILL_CALL.into()));
    for prompt in ["!profile inspect", "$custom inspect", "plain text"] {
        assert!(!is_pro_command(prompt));
        assert_eq!(rewrite_pro_prompt(prompt), None);
    }
}

#[test]
fn mapped_pro_prompt_adds_exact_thread_scoped_local_device_instruction() {
    let ticket = DeviceTicket {
        device_id: "device&1".into(),
        working_directory: PathBuf::from(r"C:\repo"),
    };
    let command = rewrite_pro_prompt("!pro review this project").expect("Pro prompt");
    let rewritten = format_local_device_prompt(&command, "thread-1", &ticket);

    assert_eq!(
        pro_conversation_scope("thread-1"),
        "codex-pro-4b0a5fefc328e6b9257bc535"
    );
    assert_eq!(
        rewritten,
        concat!(
            "$ask-chatgpt-pro [@Chrome](plugin://chrome@openai-bundled) <pro-review>\n",
            "this project\n",
            "<local-device-mcp connector=\"Simdorei Local Project Oauth\" ",
            "resource=\"https://simdorei.duckdns.org/mcp\" ",
            "device_id=\"device&amp;1\" working_directory=\"C:\\repo\" ",
            "conversation_scope=\"codex-pro-4b0a5fefc328e6b9257bc535\">\n",
            "Use only the connector named in this tag and select it explicitly.\n",
            "Use PC mode by default.\n",
            "Call list_devices, verify that device_id is online, then call select_device\n",
            "exactly once with the device_id, working_directory, and connector resource\n",
            "from this tag. The working directory identifies the project for this ticket.\n",
            "Read a file before updating it and pass its SHA-256 when writing an existing file.\n",
            "</local-device-mcp>"
        )
    );
}
