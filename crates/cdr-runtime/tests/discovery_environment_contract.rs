use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cdr_runtime::runtime_paths::{PathDiscoveryError, PathInputs, discover_inputs};

struct Fixture {
    root: tempfile::TempDir,
    home: PathBuf,
    local: PathBuf,
    executable: PathBuf,
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn inert_executable(directory: &Path) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let path = directory.join(name);
    fs::write(&path, b"inert discovery fixture; must never execute").unwrap();
    path
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("user home");
        let local = root.path().join("local apps");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&local).unwrap();
        let executable = inert_executable(&root.path().join("path bin"));
        Self {
            root,
            home,
            local,
            executable,
        }
    }

    fn environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("USERPROFILE".into(), text(&self.home)),
            ("LOCALAPPDATA".into(), text(&self.local)),
            ("PATH".into(), text(self.executable.parent().unwrap())),
            ("CODEX_HOME".into(), text(&self.root.path().join("profile"))),
        ])
    }

    fn discover(
        &self,
        environment: &BTreeMap<String, String>,
    ) -> Result<PathInputs, PathDiscoveryError> {
        discover_inputs(environment, self.root.path().to_path_buf())
    }

    fn admin(&self, environment: &BTreeMap<String, String>) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cdr-runtime"))
            .args(["--admin", "discover-codex", "--repo-root"])
            .arg(self.root.path())
            .env_clear()
            .envs(environment)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }
}

#[cfg(windows)]
#[test]
fn windows_discovery_accepts_mixed_case_os_keys() {
    let fixture = Fixture::new();
    let app = inert_executable(&fixture.local.join("OpenAI/Codex/bin/build"));
    let mut environment = fixture.environment();
    for (canonical, mixed) in [
        ("PATH", "Path"),
        ("USERPROFILE", "UserProfile"),
        ("LOCALAPPDATA", "LocalAppData"),
    ] {
        let value = environment.remove(canonical).unwrap();
        environment.insert(mixed.into(), value);
    }
    let inputs = fixture.discover(&environment).unwrap();
    assert_eq!(inputs.user_home, fixture.home);
    assert!(inputs.local_app_candidates.contains(&app));
    assert_eq!(inputs.path_candidates, [fixture.executable]);
}

#[cfg(windows)]
#[test]
fn windows_exact_os_keys_win_including_blank_values() {
    let fixture = Fixture::new();
    let mut environment = fixture.environment();
    let other = inert_executable(&fixture.root.path().join("other bin"));
    let app = inert_executable(&fixture.local.join("OpenAI/Codex/bin/build"));
    let other_local = fixture.root.path().join("other local apps");
    let other_app = inert_executable(&other_local.join("OpenAI/Codex/bin/build"));
    environment.insert("Path".into(), text(other.parent().unwrap()));
    environment.insert("userprofile".into(), "unused-home".into());
    environment.insert("localappdata".into(), text(&other_local));
    let inputs = fixture.discover(&environment).unwrap();
    assert_eq!(inputs.user_home, fixture.home);
    assert_eq!(
        inputs.path_candidates.as_slice(),
        std::slice::from_ref(&fixture.executable)
    );
    assert!(inputs.local_app_candidates.contains(&app));
    assert!(!inputs.local_app_candidates.contains(&other_app));

    environment.insert("PATH".into(), String::new());
    environment.insert("LOCALAPPDATA".into(), String::new());
    let inputs = fixture.discover(&environment).unwrap();
    assert!(!inputs.path_candidates.contains(&other));
    assert!(!inputs.local_app_candidates.contains(&other_app));
    environment.insert("USERPROFILE".into(), "  ".into());
    environment.insert("HOME".into(), text(&fixture.home));
    assert_eq!(
        fixture.discover(&environment).unwrap().user_home,
        fixture.home
    );
    environment.insert("HOME".into(), String::new());
    environment.insert("home".into(), text(&fixture.home));
    assert_eq!(
        fixture.discover(&environment).unwrap_err(),
        PathDiscoveryError::UserHomeMissing
    );
}

#[cfg(windows)]
#[test]
fn windows_identical_fallback_values_are_unambiguous() {
    let fixture = Fixture::new();
    let mut environment = fixture.environment();
    let path = environment.remove("PATH").unwrap();
    for key in ["Path", "path"] {
        environment.insert(key.into(), path.clone());
    }
    assert_eq!(
        fixture
            .discover(&environment)
            .unwrap()
            .path_candidates
            .as_slice(),
        std::slice::from_ref(&fixture.executable)
    );
    for key in ["Path", "path"] {
        environment.insert(key.into(), String::new());
    }
    assert!(fixture.discover(&environment).is_ok());
    environment.remove("USERPROFILE");
    environment.insert("UserProfile".into(), String::new());
    environment.insert("userprofile".into(), String::new());
    environment.insert("HOME".into(), text(&fixture.home));
    assert_eq!(
        fixture.discover(&environment).unwrap().user_home,
        fixture.home
    );
}

#[cfg(windows)]
#[test]
fn windows_conflicting_fallbacks_fail_without_disclosing_values() {
    let fixture = Fixture::new();
    for (canonical, first, second) in [
        ("PATH", "Path", "path"),
        ("USERPROFILE", "UserProfile", "userprofile"),
        ("HOME", "Home", "home"),
        ("LOCALAPPDATA", "LocalAppData", "localappdata"),
    ] {
        for (left, right) in [
            ("left-value", "right-value"),
            ("", "nonblank"),
            ("same", "same "),
        ] {
            let mut environment = fixture.environment();
            environment.remove(canonical);
            if canonical == "HOME" {
                environment.remove("USERPROFILE");
            }
            environment.insert(first.into(), left.into());
            environment.insert(second.into(), right.into());
            let error = fixture.discover(&environment).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("conflicting Windows environment values for {canonical}")
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn windows_unqueried_home_conflicts_do_not_override_userprofile() {
    let fixture = Fixture::new();
    let mut environment = fixture.environment();
    environment.insert("Home".into(), "left-value".into());
    environment.insert("home".into(), "right-value".into());
    assert_eq!(
        fixture.discover(&environment).unwrap().user_home,
        fixture.home
    );
}

#[cfg(not(windows))]
#[test]
fn unix_discovery_remains_case_sensitive() {
    let fixture = Fixture::new();
    let mut environment = fixture.environment();
    environment.insert("Path".into(), "left-value".into());
    environment.insert("path".into(), "right-value".into());
    assert_eq!(
        fixture
            .discover(&environment)
            .unwrap()
            .path_candidates
            .as_slice(),
        std::slice::from_ref(&fixture.executable)
    );
    environment.remove("PATH");
    assert!(
        fixture
            .discover(&environment)
            .unwrap()
            .path_candidates
            .is_empty()
    );
    environment.remove("USERPROFILE");
    environment.insert("userprofile".into(), text(&fixture.home));
    environment.insert("Home".into(), text(&fixture.home));
    assert_eq!(
        fixture.discover(&environment).unwrap_err(),
        PathDiscoveryError::UserHomeMissing
    );
}

#[test]
fn admin_discovery_obeys_platform_path_key_case() {
    let fixture = Fixture::new();
    for key in ["PATH", "Path", "path"] {
        let mut environment = fixture.environment();
        let path = environment.remove("PATH").unwrap();
        environment.insert(key.into(), path);
        let output = fixture.admin(&environment);
        let expected_success = cfg!(windows) || key == "PATH";
        assert_eq!(
            output.status.success(),
            expected_success,
            "{key}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if expected_success {
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                text(&fixture.executable)
            );
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("no usable Codex executable"));
        }
    }
}

#[cfg(windows)]
#[test]
fn windows_admin_merge_preserves_exact_keys_and_rejects_conflicts() {
    let fixture = Fixture::new();
    let other = inert_executable(&fixture.root.path().join("project bin"));
    for (inherited_key, project_key, expected) in [
        ("Path", "path", None),
        ("Path", "PATH", Some(&other)),
        ("PATH", "PATH", Some(&fixture.executable)),
    ] {
        let mut environment = fixture.environment();
        let path = environment.remove("PATH").unwrap();
        environment.insert(inherited_key.into(), path);
        fs::write(
            fixture.root.path().join(".env"),
            format!("{project_key}={}\n", other.parent().unwrap().display()),
        )
        .unwrap();
        let output = fixture.admin(&environment);
        if let Some(executable) = expected {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                text(executable)
            );
        } else {
            assert!(!output.status.success());
            assert_eq!(
                String::from_utf8(output.stderr).unwrap().trim(),
                "ERROR: conflicting Windows environment values for PATH"
            );
        }
    }
}

#[test]
fn admin_discovery_still_rejects_missing_executable() {
    let fixture = Fixture::new();
    let mut environment = fixture.environment();
    environment.insert("PATH".into(), text(&fixture.root.path().join("missing")));
    let output = fixture.admin(&environment);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no usable Codex executable"));
}
