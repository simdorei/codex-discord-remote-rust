//! Port of the container hardening contracts, retaining the original isolation boundaries.
use std::{fs, path::Path};
fn read(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../remote_mcp_server")
            .join(name),
    )
    .unwrap()
    .replace("\r\n", "\n")
}
#[test]
fn default_container_is_native_and_both_build_images_are_digest_pinned() {
    let text = read("Dockerfile");
    assert!(text.contains("FROM rust:1.97.1-bookworm@sha256:"));
    assert!(text.contains("FROM debian:bookworm-slim@sha256:"));
    for line in text.lines().filter(|line| line.starts_with("FROM ")) {
        let digest = line
            .split("@sha256:")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|b| b.is_ascii_hexdigit()));
    }
    assert!(text.contains("cargo build --locked --release -p cdr-mcp-server --bin cdr-mcp-server"));
    assert!(text.contains("ENTRYPOINT [\"/usr/local/bin/cdr-mcp-server\"]"));
    assert!(!text.to_lowercase().contains("python"));
    assert!(!text.contains("uv sync"));
    assert_eq!(text, read("Dockerfile.rust"));
}
#[test]
fn gateway_declares_fixed_non_root_user_and_tls_certificates() {
    let text = read("Dockerfile");
    for expected in [
        "groupadd --gid 10001 simdorei",
        "useradd --uid 10001",
        "USER 10001:10001",
        "ca-certificates",
    ] {
        assert!(text.contains(expected), "{expected}");
    }
}
#[test]
fn compose_preserves_oauth_volume_initialization_before_gateway() {
    let text = read("compose.yaml");
    for expected in [
        "oauth-data-init:",
        "user: \"0:0\"",
        "chown -R --no-dereference 10001:10001 /data",
        "condition: service_completed_successfully",
    ] {
        assert!(text.contains(expected));
    }
}
#[test]
fn gateway_privileges_write_paths_and_native_healthcheck_are_bounded() {
    let text = read("compose.yaml");
    for expected in [
        "user: \"10001:10001\"",
        "cap_drop:\n      - ALL",
        "no-new-privileges:true",
        "read_only: true",
        "/tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777",
        "pids_limit: 256",
        "init: true",
        "127.0.0.1:8030:8030",
        "- /usr/local/bin/cdr-mcp-server",
        "- --healthcheck",
    ] {
        assert!(text.contains(expected), "{expected}");
    }
    assert!(!text.to_lowercase().contains("python"));
    assert_eq!(text, read("compose.rust.yaml"));
}
