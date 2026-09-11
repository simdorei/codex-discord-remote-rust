#[path = "support/installer_native.rs"]
mod fixture;
use std::fs;

#[test]
fn only_explicit_codex_selection_becomes_a_persistent_pin() {
    let platforms = if cfg!(windows) {
        vec![true, false]
    } else {
        vec![false]
    };
    for windows in platforms {
        for selection in ["path", "inherited", "explicit"] {
            let root = tempfile::tempdir().unwrap();
            let codex = fixture::seed(root.path());
            let explicit = (selection == "explicit").then_some(codex.as_path());
            let mut command = fixture::command(root.path(), windows, explicit, false);
            let path = std::env::join_paths(
                std::iter::once(codex.parent().unwrap().to_path_buf())
                    .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
            )
            .unwrap();
            command
                .arg(if windows {
                    "-SkipCodexPlugin"
                } else {
                    "--skip-codex-plugin"
                })
                .env("PATH", path)
                .env("CODEX_HOME", root.path().join("codex-home"))
                .env_remove("CODEX_EXE");
            if selection == "inherited" {
                command.env("CODEX_EXE", &codex);
            }
            let out = fixture::bounded(command);
            let result = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(out.status.success(), "{windows}/{selection}: {result}");
            let env = fs::read_to_string(root.path().join(".env")).unwrap();
            let value = env
                .lines()
                .find_map(|l| l.strip_prefix("CODEX_EXE="))
                .unwrap();
            if selection == "explicit" {
                assert_eq!(
                    value.replace('\\', "/"),
                    codex.to_str().unwrap().replace('\\', "/")
                );
            } else {
                assert_eq!(value, "");
                let expected = if selection == "path" {
                    "PATH-discovered Codex command was not saved"
                } else {
                    "Inherited CODEX_EXE was not saved"
                };
                assert!(result.contains(expected), "{windows}/{selection}: {result}");
            }
            assert!(env.lines().any(|l| l == "KEEP=한글"));
            assert!(!root.path().join("cli-calls.jsonl").exists());
        }
    }
}
