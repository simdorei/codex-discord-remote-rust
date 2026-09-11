#[path = "../examples/support/maintenance_pins.rs"]
mod pins;
use cdr_runtime::runtime_paths::{PathInputs, RuntimePaths};
use std::{collections::BTreeMap, fs};
#[test]
fn environment_overrides_rejected_before_database_or_rpc_without_operator_opt_in() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("codex-home");
    fs::create_dir(&home).unwrap();
    let state = home.join("state_5.sqlite");
    let mirror = root.path().join("discord_mirror.sqlite");
    let codex = root.path().join("codex.exe");
    let bytes = b"not a SQLite database; must never be queried";
    for path in [&state, &mirror, &codex] {
        fs::write(path, bytes).unwrap();
    }
    let base = BTreeMap::from([
        ("CODEX_HOME".into(), home.to_str().unwrap().into()),
        ("CODEX_STATE_DB".into(), state.to_str().unwrap().into()),
        (
            "CODEX_DISCORD_ROOT".into(),
            root.path().to_str().unwrap().into(),
        ),
        (
            "CODEX_DISCORD_MIRROR_DB".into(),
            mirror.to_str().unwrap().into(),
        ),
        ("CODEX_EXE".into(), codex.to_str().unwrap().into()),
    ]);
    let inputs = PathInputs::new(root.path().into(), root.path().into(), vec![], vec![]);
    let approved = RuntimePaths::resolve(&base, &inputs).unwrap();
    pins::verify(&approved, root.path(), &home, &state, &mirror).unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let database = foreign.path().join("fixture.sqlite");
    fs::write(&database, bytes).unwrap();
    for (key, path, error) in [
        (
            "CODEX_STATE_DB",
            database.as_path(),
            "approved Codex home/state DB mismatch",
        ),
        (
            "CODEX_HOME",
            foreign.path(),
            "approved Codex home/state DB mismatch",
        ),
        (
            "CODEX_DISCORD_MIRROR_DB",
            database.as_path(),
            "maintenance DB mismatch",
        ),
        (
            "CODEX_DISCORD_ROOT",
            foreign.path(),
            "legacy mutex root spelling mismatch",
        ),
    ] {
        let mut environment = base.clone();
        environment.insert(key.into(), path.to_str().unwrap().into());
        let resolved = RuntimePaths::resolve(&environment, &inputs).unwrap();
        assert!(
            pins::verify(&resolved, root.path(), &home, &state, &mirror)
                .unwrap_err()
                .to_string()
                .contains(error)
        );
        for path in [&database, &mirror, &state] {
            assert_eq!(fs::read(path).unwrap(), bytes);
            assert!(!path.with_extension("sqlite-wal").exists());
        }
    }
    assert_eq!(fs::read_dir(foreign.path()).unwrap().count(), 1);
}
