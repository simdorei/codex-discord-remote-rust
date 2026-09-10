use std::collections::BTreeMap;
use std::fs;
use std::thread;
use std::time::Duration;

use cdr_runtime::runtime_paths::{
    PathDiscoveryError, PathInputs, PathSource, RuntimePathError, RuntimePaths, discover_inputs,
};

fn touch(path: &std::path::Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"fixture").unwrap();
}

#[test]
fn explicit_paths_win_and_the_existing_python_mirror_location_is_the_default() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let home = temp.path().join("home");
    let explicit_exe = temp.path().join("current-codex.exe");
    let explicit_state = temp.path().join("state.sqlite");
    touch(&explicit_exe);
    touch(&explicit_state);
    let env = BTreeMap::from([
        (
            "CODEX_EXE".into(),
            explicit_exe.to_string_lossy().into_owned(),
        ),
        (
            "CODEX_STATE_DB".into(),
            explicit_state.to_string_lossy().into_owned(),
        ),
    ]);

    let paths = RuntimePaths::resolve(
        &env,
        &PathInputs::new(root.clone(), home, Vec::new(), Vec::new()),
    )
    .unwrap();

    assert_eq!(paths.codex_exe, explicit_exe);
    assert_eq!(paths.codex_exe_source, PathSource::Environment);
    assert_eq!(paths.state_db, explicit_state);
    assert_eq!(paths.mirror_db, root.join("discord_mirror.sqlite"));
    assert_eq!(
        paths.bridge_state,
        paths.codex_home.join("codex_desktop_bridge_state.json")
    );
    assert_eq!(
        paths.attachment_dir,
        root.join(".codex-discord-attachments")
    );
}

#[test]
fn sandbox_bin_is_preferred_and_latest_codex_state_db_is_selected() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let home = temp.path().join("home");
    let codex_home = home.join(".codex");
    let sandbox = codex_home.join(".sandbox-bin/codex.exe");
    let old_state = codex_home.join("state_4.sqlite");
    let new_state = codex_home.join("state_5.sqlite");
    let path_exe = temp.path().join("path-bin/codex.exe");
    touch(&sandbox);
    touch(&path_exe);
    touch(&old_state);
    thread::sleep(Duration::from_millis(20));
    touch(&new_state);

    let paths = RuntimePaths::resolve(
        &BTreeMap::new(),
        &PathInputs::new(root, home, Vec::new(), vec![path_exe]),
    )
    .unwrap();

    assert_eq!(paths.codex_exe, sandbox);
    assert_eq!(paths.codex_exe_source, PathSource::SandboxBin);
    assert_eq!(paths.state_db, new_state);
}

#[test]
fn blank_codex_exe_uses_newest_local_app_before_sandbox_and_path() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("runtime");
    let home = temp.path().join("home");
    let sandbox = home.join(".codex/.sandbox-bin/codex.exe");
    let old_local = temp.path().join("OpenAI/Codex/bin/old/codex.exe");
    let new_local = temp.path().join("OpenAI/Codex/bin/new/codex.exe");
    let path_exe = temp.path().join("path-bin/codex.exe");
    touch(&sandbox);
    touch(&old_local);
    thread::sleep(Duration::from_millis(20));
    touch(&new_local);
    touch(&path_exe);
    let env = BTreeMap::from([("CODEX_EXE".into(), "  \"\"  ".into())]);

    let paths = RuntimePaths::resolve(
        &env,
        &PathInputs::new(
            root,
            home,
            vec![old_local, new_local.clone()],
            vec![path_exe],
        ),
    )
    .unwrap();

    assert_eq!(paths.codex_exe, new_local);
    assert_eq!(paths.codex_exe_source, PathSource::LocalAppBin);
}

#[test]
fn newest_local_app_binary_beats_path_and_windowsapps_aliases_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let old = temp.path().join("OpenAI/Codex/bin/old/codex.exe");
    let new = temp.path().join("OpenAI/Codex/bin/new/codex.exe");
    let windowsapps = temp.path().join("WindowsApps/codex.exe");
    touch(&old);
    thread::sleep(Duration::from_millis(20));
    touch(&new);
    touch(&windowsapps);
    let inputs = PathInputs::new(
        temp.path().join("runtime"),
        temp.path().join("home"),
        vec![old, new.clone()],
        vec![windowsapps],
    );

    let paths = RuntimePaths::resolve(&BTreeMap::new(), &inputs).unwrap();
    assert_eq!(paths.codex_exe, new);
    assert_eq!(paths.codex_exe_source, PathSource::LocalAppBin);

    let only_alias = PathInputs::new(
        temp.path().join("runtime"),
        temp.path().join("other-home"),
        Vec::new(),
        vec![temp.path().join("WindowsApps/codex.exe")],
    );
    assert_eq!(
        RuntimePaths::resolve(&BTreeMap::new(), &only_alias).unwrap_err(),
        RuntimePathError::WindowsAppsAliasOnly
    );
}

#[test]
fn configured_missing_executable_is_not_silently_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing.exe");
    let fallback = temp.path().join("fallback/codex.exe");
    touch(&fallback);
    let env = BTreeMap::from([("CODEX_EXE".into(), missing.to_string_lossy().into_owned())]);

    let error = RuntimePaths::resolve(
        &env,
        &PathInputs::new(
            temp.path().join("runtime"),
            temp.path().join("home"),
            vec![fallback],
            Vec::new(),
        ),
    )
    .unwrap_err();

    assert_eq!(
        error,
        RuntimePathError::ConfiguredExecutableMissing(missing)
    );
}

#[test]
fn current_machine_discovery_uses_loaded_env_including_local_app_and_path() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let local = temp.path().join("local");
    let app_exe = local.join("OpenAI/Codex/bin/build-1/codex.exe");
    let path_dir = temp.path().join("path-bin");
    let path_exe = path_dir.join("codex.exe");
    touch(&app_exe);
    touch(&path_exe);
    let env = BTreeMap::from([
        ("USERPROFILE".into(), home.to_string_lossy().into_owned()),
        ("LOCALAPPDATA".into(), local.to_string_lossy().into_owned()),
        ("PATH".into(), path_dir.to_string_lossy().into_owned()),
    ]);

    let inputs = discover_inputs(&env, temp.path().join("runtime")).unwrap();
    assert!(inputs.local_app_candidates.contains(&app_exe));
    assert!(inputs.path_candidates.contains(&path_exe));

    let missing = discover_inputs(&BTreeMap::new(), temp.path().join("runtime")).unwrap_err();
    assert_eq!(missing, PathDiscoveryError::UserHomeMissing);
}
