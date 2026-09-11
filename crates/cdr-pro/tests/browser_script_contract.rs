use std::{fs, path::Path, process::Command};
#[test]
fn browser_glue_contracts_run_directly_in_node_without_a_python_wrapper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = fs::read_dir(root.join("tests/browser"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".test.mjs")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty());
    let output = Command::new("node")
        .args(["--test", "--test-reporter=tap"])
        .args(paths)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("# fail 0"), "{stdout}");
}
