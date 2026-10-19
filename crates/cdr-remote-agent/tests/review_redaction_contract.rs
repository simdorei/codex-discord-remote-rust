use std::fs;

use cdr_remote_agent::files::ProjectFileAccess;
use cdr_remote_protocol::message::ReadFileOutput;
use serde_json::json;

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const API: &str = "InitializeProcThreadAttributeList";

#[test]
fn rr13_public_ffi_signature_name_does_not_whitelist_arbitrary_string_keys() {
    let opaque = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCDEF";
    let text =
        format!("signatures = {{\n    '{API}': (w.BOOL, []),\n    '{opaque}': (w.BOOL, []),\n}}\n");
    let output = read("process.py", &text, 1, 100);
    assert!(output.content.contains(API));
    assert!(!output.content.contains(opaque));
    assert!(output.redacted);
}

#[test]
fn rr11_utf8_bom_json_keeps_raw_text_and_file_hash_evidence() {
    let text = format!("\u{feff}{}", json!({"path":"code.py", "sha256": HASH}));
    let output = read("candidate.json", &text, 1, 100);
    assert_eq!(output.content, text);
    assert!(!output.redacted);
}

#[test]
fn rr12_python_bare_api_reference_is_an_identifier_not_a_secret() {
    let text = format!("initialize = kernel.{API}\n");
    let output = read("process.py", &text, 1, 100);
    assert_eq!(output.content, text.trim_end());
    assert!(!output.redacted);
}

fn read(name: &str, content: &str, start: u64, count: u16) -> ReadFileOutput {
    let directory = tempfile::tempdir().expect("tempdir");
    fs::write(directory.path().join(name), content).expect("fixture");
    ProjectFileAccess::open(directory.path())
        .expect("access")
        .read_file(name, start, count)
        .expect("read")
}

#[test]
fn rr1_python_api_and_test_identifiers_remain_readable() {
    let code = concat!(
        "kernel.InitializeProcThreadAttributeList.argtypes = [ctypes.c_void_p]\n",
        "def test_receipt_requires_all_manifest_assets_and_tools():\n",
        "    kernel.InitializeProcThreadAttributeList(None, 1, 0, size)\n",
    );
    let output = read("process.py", code, 1, 100);
    assert_eq!(output.content, code.trim_end());
    assert!(!output.redacted);
}

#[test]
fn rr2_file_digest_and_source_hash_map_remain_readable() {
    let document = json!({
        "schema": "review.candidate.v1",
        "repository": "C:/work/content-production-skills",
        "head": "0123456789abcdef0123456789abcdef01234567",
        "files": [{"source": "code.py", "snapshot": "candidate/code.py", "sha256": HASH}],
        "source_hashes": {"C:/work/code.py": HASH}
    });
    let text = serde_json::to_string_pretty(&document).unwrap();
    let output = read("candidate.json", &text, 1, 100);
    assert_eq!(output.content, text);
    assert!(!output.redacted);
}

#[test]
fn rr3_same_digest_in_credential_context_still_redacts() {
    let document = json!({
        "files": [{"path": "code.py", "sha256": HASH}],
        "api_key": HASH,
        "token": HASH,
        "credentials": {"source_hashes": {"code.py": HASH}},
        "password": API
    });
    let output = read(
        "candidate.json",
        &serde_json::to_string_pretty(&document).unwrap(),
        1,
        100,
    );
    let result: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["files"][0]["sha256"], HASH);
    for key in ["api_key", "token", "password"] {
        assert_eq!(result[key], "[REDACTED]", "credential field {key}");
    }
    assert_eq!(
        result["credentials"]["source_hashes"]["code.py"],
        "[REDACTED]"
    );
    assert!(output.redacted);
}

#[test]
fn rr4_unknown_hex_and_untyped_hashes_still_redact() {
    for (name, text) in [
        ("notes.txt", HASH.to_owned()),
        ("settings.json", json!({"sha256": HASH}).to_string()),
        ("settings.json", json!({"value": HASH}).to_string()),
        (
            "notes.txt",
            json!({"path": "code.py", "sha256": HASH}).to_string(),
        ),
        (
            "broken.json",
            format!("{{\"path\":\"code.py\",\"sha256\":\"{HASH}\""),
        ),
    ] {
        let output = read(name, &text, 1, 100);
        assert!(!output.content.contains(HASH), "opaque value in {name}");
        assert!(output.redacted);
    }
}

#[test]
fn rr5_strings_and_comments_do_not_gain_identifier_exemption() {
    let code = format!(
        "value = '{API}'\n# kernel.{API}(value)\ntext = \"\"\"\ndef {API}():\n\"\"\"\nkernel.{API}(value)\n"
    );
    let output = read("process.py", &code, 1, 100);
    assert_eq!(output.content.matches(API).count(), 1);
    assert!(output.content.ends_with(&format!("kernel.{API}(value)")));
    assert!(
        read("process.py", &code, 4, 1)
            .content
            .contains("[REDACTED]")
    );
}

#[test]
fn rr6_partial_json_reads_keep_parent_credential_context() {
    let document = json!({"credentials": {"source_hashes": {"code.py": HASH}}});
    let text = serde_json::to_string_pretty(&document).unwrap();
    let line = text.lines().position(|line| line.contains(HASH)).unwrap() as u64 + 1;
    let output = read("candidate.json", &text, line, 1);
    assert!(!output.content.contains(HASH));
    assert!(output.redacted);
    assert_eq!(output.start_line, line);
    assert_eq!(output.end_line, line);
    assert_eq!(output.total_lines, text.lines().count() as u64);
}

#[test]
fn rr7_multiline_private_key_redacts_before_slicing_without_moving_lines() {
    let key_body = "QUFBQUFB";
    let text = format!(
        "before\n-----BEGIN PRIVATE KEY-----\n{key_body}\n-----END PRIVATE KEY-----\nafter\n"
    );
    let body = read("notes.txt", &text, 3, 1);
    assert!(!body.content.contains(key_body));
    assert_eq!(body.start_line, 3);
    assert_eq!(body.total_lines, 5);
    assert_eq!(read("notes.txt", &text, 5, 1).content, "after");
}

#[test]
fn rr8_path_literals_and_manifest_paths_are_not_opaque_tokens() {
    let path = "node_modules/@ffprobe-installer/win32-x64/ffprobe.exe";
    let code = format!("probe = ENGINE / \"{path}\"\n");
    assert_eq!(read("process.py", &code, 1, 100).content, code.trim_end());
    let text = json!({"files":[{"path":"candidate-r1/longform-harness-review-20261007/code.py","sha256":HASH}]}).to_string();
    assert_eq!(read("candidate.json", &text, 1, 100).content, text);
}

#[test]
fn rr9_digest_permission_does_not_escape_one_json_value_or_sensitive_ancestor() {
    let text = json!({
        "files": [{"path":"code.py", "sha256": HASH}],
        "unknown": HASH,
        "secrets": {"files": [{"path":"code.py", "sha256":HASH}]},
        "api_key": "node_modules/@ffprobe-installer/win32-x64/ffprobe.exe"
    })
    .to_string();
    let output = read("candidate.json", &text, 1, 100);
    let result: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["files"][0]["sha256"], HASH);
    assert_eq!(result["unknown"], "[REDACTED]");
    assert_eq!(result["secrets"]["files"][0]["sha256"], "[REDACTED]");
    assert_eq!(result["api_key"], "[REDACTED]");
}

#[test]
fn rr10_synthetic_provider_keys_and_opaque_tokens_remain_protected() {
    let opaque = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789ABCDEF";
    let text = format!(
        "AWS=AKIAABCDEFGHIJKLMNOP\nAuthorization: Bearer {opaque}\nvalue={opaque}\napi_key='{HASH}'\n"
    );
    let output = read("process.py", &text, 1, 100);
    for secret in ["AKIAABCDEFGHIJKLMNOP", opaque, HASH] {
        assert!(!output.content.contains(secret));
    }
    assert!(output.redacted);
}
