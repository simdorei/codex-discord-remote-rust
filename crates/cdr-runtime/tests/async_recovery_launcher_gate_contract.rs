#![cfg(windows)]
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn sha(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16384];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

fn declaration(root: &Path, binary: &Path, version: i64) -> PathBuf {
    let path = root.join("capabilities.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "protocol":"cdr-artifact-capabilities-v1", "artifact_sha256":sha(binary),
            "async_resolution_max_format":version, "async_recovery_policy_max_format":1
        }))
        .unwrap(),
    )
    .unwrap();
    path
}

fn probe(root: &Path, binary: &Path, database: &Path, manifest: &Path, pinned: &str) -> Output {
    let module = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/candidates/async-recovery-v1/CdrAsyncRecoveryCompatibility.ps1")
        .canonicalize()
        .unwrap();
    let script = format!(
        "$ErrorActionPreference='Stop'; . {}; try {{ $r=Assert-CdrAsyncRecoveryCompatibility -CandidatePath {} -DatabasePath {} -CapabilityManifestPath {} -ExpectedCandidateSha256 '{}' -ExpectedCapabilitySha256 '{}'; $r | ConvertTo-Json -Compress; exit 0 }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 9 }}",
        quoted(&module),
        quoted(binary),
        quoted(database),
        quoted(manifest),
        pinned,
        sha(manifest),
    );
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .current_dir(root)
        .output()
        .unwrap()
}

#[test]
fn incompatible_or_unpinned_artifact_is_rejected_before_it_can_execute() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("not-an-executable.exe");
    std::fs::write(&binary, b"non-executable fixture").unwrap();
    let db = temp.path().join("absent.sqlite");
    let manifest = declaration(temp.path(), &binary, 0);
    let output = probe(temp.path(), &binary, &db, &manifest, &sha(&binary));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("capability is unsupported"),
        "{output:?}"
    );
    let output = probe(temp.path(), &binary, &db, &manifest, &"0".repeat(64));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("hash mismatch"),
        "{output:?}"
    );
    assert!(!db.exists());
}

#[test]
fn pinned_real_candidate_reads_requirements_without_starting_gateway_or_changing_database() {
    let temp = tempfile::tempdir().unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_cdr-runtime"));
    let db = temp.path().join("store.sqlite");
    let connection = cdr_store::schema::open_initialized(&db).unwrap();
    connection.execute("INSERT INTO cdr_runtime_capability_requirements(component,format_version) VALUES('async_resolution',1)",[]).unwrap();
    drop(connection);
    let manifest = declaration(temp.path(), binary, 1);
    let before = std::fs::read(&db).unwrap();
    let output = probe(temp.path(), binary, &db, &manifest, &sha(binary));
    assert!(output.status.success(), "{output:?}");
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["compatible"], true);
    assert_eq!(response["admission_authorized"], false);
    assert_eq!(std::fs::read(&db).unwrap(), before);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute(
            "UPDATE cdr_runtime_capability_requirements SET format_version=2",
            [],
        )
        .unwrap();
    drop(connection);
    let before = std::fs::read(&db).unwrap();
    let output = probe(temp.path(), binary, &db, &manifest, &sha(binary));
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("does not support"),
        "{output:?}"
    );
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

#[test]
fn armed_wrapper_checks_real_candidate_environment_before_its_launch_callback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let binary = Path::new(env!("CARGO_BIN_EXE_cdr-runtime"));
    let db = root.join("store.sqlite");
    drop(cdr_store::schema::open_initialized(&db).unwrap());
    let before = std::fs::read(&db).unwrap();
    let env = root.join("selected.env");
    std::fs::write(&env, "CODEX_DISCORD_MIRROR_DB=store.sqlite\n").unwrap();
    let caps = declaration(root, binary, 1);
    let contract = root.join(".codex_discord_rust.compatibility.json");
    std::fs::write(
        &contract,
        serde_json::to_vec(&serde_json::json!({
            "protocol":"cdr-runtime-launch-v1","root":root,
            "candidate_path":binary,"candidate_sha256":sha(binary),
            "environment_path":env,"environment_sha256":sha(&env),
            "database_path":db,"capability_path":caps,"capability_sha256":sha(&caps)
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join(".codex_discord_rust.compatibility.required"),
        sha(&contract),
    )
    .unwrap();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let control = repo.join("codex-discord-rust-control.ps1");
    let module =
        repo.join("scripts/candidates/async-recovery-v1/CdrAsyncRecoveryCompatibility.ps1");
    let wrapper =
        repo.join("scripts/candidates/async-recovery-v1/CdrRuntimeLaunchCompatibility.ps1");
    let script = format!(
        "$ErrorActionPreference='Stop'; . {}; . {}; . {}; $g=Enter-CdrControl -Root {}; try {{ Invoke-CdrCheckedRuntimeLaunch -RepoRoot {} -CandidatePath {} -EnvironmentPath {} -ControlGuard $g -Launch {{ [Console]::WriteLine('checked_launch_callback') }} }} finally {{ $g.Dispose() }}",
        quoted(&control),
        quoted(&module),
        quoted(&wrapper),
        quoted(root),
        quoted(root),
        quoted(binary),
        quoted(&env),
    );
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .env_remove("CODEX_DISCORD_ROOT")
        .env_remove("CODEX_DISCORD_MIRROR_DB")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "checked_launch_callback"
    );
    assert_eq!(std::fs::read(db).unwrap(), before);
}
