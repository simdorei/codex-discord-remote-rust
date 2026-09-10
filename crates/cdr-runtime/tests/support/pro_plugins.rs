use serde_json::json;
use std::path::{Path, PathBuf};

pub fn installed(root: &Path) -> PathBuf {
    let remote = root.join("plugins/codex-discord-remote");
    let chrome = root.join("chrome");
    std::fs::create_dir_all(remote.join(".codex-plugin")).unwrap();
    std::fs::create_dir_all(&chrome).unwrap();
    std::fs::write(
        remote.join(".codex-plugin/plugin.json"),
        r#"{"version":"fixture-1"}"#,
    )
    .unwrap();
    std::fs::write(chrome.join("plugin.txt"), "fixture").unwrap();
    let inventory = root.join("inventory.json");
    std::fs::write(&inventory,json!({"installed":[
        {"pluginId":"codex-discord-remote@codex-discord-remote","installed":true,"enabled":true,"version":"fixture-1","source":{"path":remote}},
        {"pluginId":"chrome@openai-bundled","installed":true,"enabled":true,"version":"fixture-1","source":{"path":chrome}}
    ]}).to_string()).unwrap();
    executable(root, &inventory)
}

#[cfg(windows)]
fn executable(root: &Path, inventory: &Path) -> PathBuf {
    let script = root.join("codex-fixture.cmd");
    std::fs::write(&script,format!("@echo off\r\nif not \"%~1 %~2 %~3\"==\"plugin list --json\" exit /b 2\r\ntype \"{}\"\r\n",inventory.display())).unwrap();
    script
}

#[cfg(not(windows))]
fn executable(root: &Path, inventory: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = root.join("codex-fixture");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1 $2 $3\" = 'plugin list --json' ] || exit 2\ncat '{}'\n",
            inventory.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    script
}
