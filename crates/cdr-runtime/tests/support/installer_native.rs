#![allow(dead_code)]
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn shell() -> Command {
    #[cfg(windows)]
    {
        let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .filter(|p| p.join("git.exe").is_file())
            .filter_map(|p| p.parent().map(|p| p.join("bin/bash.exe")))
            .find(|p| p.is_file())
            .expect("Git Bash is required");
        Command::new(path)
    }
    #[cfg(not(windows))]
    {
        Command::new("sh")
    }
}

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
pub fn seed(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("scripts")).unwrap();
    for entry in fs::read_dir(repo().join("scripts")).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|v| v == "psm1") {
            fs::copy(entry.path(), root.join("scripts").join(entry.file_name())).unwrap();
        }
    }
    for file in ["install.ps1", "install.sh"] {
        fs::write(
            root.join(file),
            fs::read_to_string(repo().join(file))
                .unwrap()
                .replace("\r\n", "\n"),
        )
        .unwrap();
    }
    fs::write(root.join(".env"), "KEEP=한글\nCODEX_EXE=\n").unwrap();
    fs::create_dir_all(root.join(".agents/plugins")).unwrap();
    fs::copy(
        repo().join(".agents/plugins/marketplace.json"),
        root.join(".agents/plugins/marketplace.json"),
    )
    .unwrap();
    let plugin = root.join("plugins/codex-discord-remote/.codex-plugin");
    fs::create_dir_all(&plugin).unwrap();
    fs::copy(
        repo().join("plugins/codex-discord-remote/.codex-plugin/plugin.json"),
        plugin.join("plugin.json"),
    )
    .unwrap();
    let directory = root.join("fixture-bin");
    fs::create_dir(&directory).unwrap();
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    fs::copy(
        env!("CARGO_BIN_EXE_cdr-runtime"),
        directory.join(format!("cdr-runtime{suffix}")),
    )
    .unwrap();
    let helper = Path::new(env!("CARGO_BIN_EXE_cdr-runtime"))
        .parent()
        .unwrap()
        .join(format!("cdr-pro-helper{suffix}"));
    assert!(
        helper.is_file(),
        "missing QA helper at {}; build cdr-pro-helper from this checkout with the same --target-dir/profile as cdr-runtime before running installer tests",
        helper.display()
    );
    fs::copy(&helper, directory.join("cdr-pro-helper.exe"))
        .unwrap_or_else(|error| panic!("could not copy QA helper {}: {error}", helper.display()));
    // POSIX wrapper path has no extension, including when exercised through Git Bash.
    fs::copy(
        directory.join("cdr-pro-helper.exe"),
        directory.join("cdr-pro-helper"),
    )
    .unwrap();
    let codex = directory.join(format!("codex{suffix}"));
    fs::copy(env!("CARGO_BIN_EXE_cdr-offline-soak"), &codex).unwrap();
    codex
}

pub fn inventories(root: &Path, case: &str) {
    let version: Value = serde_json::from_slice(
        &fs::read(root.join("plugins/codex-discord-remote/.codex-plugin/plugin.json")).unwrap(),
    )
    .unwrap();
    let market = json!({"name":"codex-discord-remote","root":root});
    let plugin = json!({"pluginId":"codex-discord-remote@codex-discord-remote","version":version["version"],"installed":true,"enabled":true});
    let mut markets = json!({"marketplaces":[market.clone()]});
    let mut plugins = json!({"installed":[plugin.clone()]});
    match case {
        "marketplace_missing" => markets["marketplaces"] = json!([]),
        "empty_object" => {
            markets = json!({});
            plugins = json!({});
        }
        "wrong_root" => markets["marketplaces"][0]["root"] = json!(root.join("wrong")),
        "duplicate_marketplace" => markets["marketplaces"] = json!([market.clone(), market]),
        "plugin_list_empty" => plugins["installed"] = json!([]),
        "duplicate_plugin" => plugins["installed"] = json!([plugin.clone(), plugin]),
        "plugin_not_installed" => plugins["installed"][0]["installed"] = json!(false),
        "plugin_disabled" => plugins["installed"][0]["enabled"] = json!(false),
        "version_mismatch" => plugins["installed"][0]["version"] = json!("0.0.0-stale"),
        "normal" | "malformed_json" => {}
        _ => panic!("unknown inventory case"),
    }
    fs::write(
        root.join("marketplaces.json"),
        if case == "malformed_json" {
            "{not-json".into()
        } else {
            markets.to_string()
        },
    )
    .unwrap();
    fs::write(root.join("plugins.json"), plugins.to_string()).unwrap();
}

pub fn run(
    root: &Path,
    windows: bool,
    codex: &Path,
    configure: impl FnOnce(&mut Command),
) -> Output {
    let mut command = command(root, windows, Some(codex), true);
    configure(&mut command);
    bounded(command)
}

pub fn command(root: &Path, windows: bool, codex: Option<&Path>, skip_env: bool) -> Command {
    let binary = root.join(if cfg!(windows) {
        "fixture-bin/cdr-runtime.exe"
    } else {
        "fixture-bin/cdr-runtime"
    });
    let mut command = if windows {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(root.join("install.ps1"))
            .args(["-SkipBuild", "-BinaryPath"])
            .arg(&binary);
        if skip_env {
            c.arg("-SkipEnvFile");
        }
        if let Some(codex) = codex {
            c.arg("-CodexExe").arg(codex);
        }
        c
    } else {
        let mut c = shell();
        c.arg(root.join("install.sh").to_str().unwrap().replace('\\', "/"))
            .args(["--skip-build", "--binary-path"])
            .arg(binary.to_str().unwrap().replace('\\', "/"));
        if skip_env {
            c.arg("--skip-env-file");
        }
        if let Some(codex) = codex {
            c.arg("--codex-exe")
                .arg(codex.to_str().unwrap().replace('\\', "/"));
        }
        c
    };
    command
        .current_dir(root)
        .env("CDR_CODEX_CLI_FIXTURE_ROOT", root)
        .env_remove("CDR_CLI_FAIL")
        .env_remove("CDR_CLI_WARN")
        .env_remove("CDR_CLI_LARGE");
    command
}

pub fn bounded(mut command: Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = |mut stream: Box<dyn std::io::Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            bytes
        })
    };
    let out = reader(Box::new(stdout));
    let err = reader(Box::new(stderr));
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("native installer fixture timed out");
        }
        thread::sleep(Duration::from_millis(25));
    };
    Output {
        status,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}

pub const INVALID: &[&str] = &[
    "marketplace_missing",
    "empty_object",
    "malformed_json",
    "wrong_root",
    "duplicate_marketplace",
    "plugin_list_empty",
    "duplicate_plugin",
    "plugin_not_installed",
    "plugin_disabled",
    "version_mismatch",
];
