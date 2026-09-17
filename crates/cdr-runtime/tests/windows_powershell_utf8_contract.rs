#![cfg(windows)]

#[path = "support/powershell_utf8.rs"]
mod powershell;

use std::fs;

#[test]
fn unicode_stdout_and_stderr_round_trip_without_lossy_decoding() {
    let expected = "한글 경로와 공백 ' 따옴표 日本語 😀";
    let output = powershell::command(
        "[Console]::WriteLine($env:CDR_TEST_TEXT); [Console]::Error.WriteLine($env:CDR_TEST_TEXT)",
    )
    .env("CDR_TEST_TEXT", expected)
    .output()
    .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    assert_eq!(String::from_utf8(output.stderr).unwrap().trim(), expected);
}

#[test]
fn script_path_and_arguments_are_data_and_explicit_failure_exit_is_preserved() {
    let root = tempfile::Builder::new()
        .prefix("한글 fixture' ")
        .tempdir()
        .unwrap();
    let caller = tempfile::tempdir().unwrap();
    let script = root.path().join("실행 script' name.ps1");
    fs::write(
        &script,
        "param([string]$Value)\nWrite-Output $Value\nexit 37\n",
    )
    .unwrap();
    let expected = "문자열'; throw 'MUST_NOT_EXECUTE";
    let output =
        powershell::command("& $env:CDR_TEST_SCRIPT -Value $env:CDR_TEST_TEXT; exit $LASTEXITCODE")
            .env("CDR_TEST_SCRIPT", &script)
            .env("CDR_TEST_TEXT", expected)
            .current_dir(caller.path())
            .output()
            .unwrap();
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), expected);
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_dir(caller.path()).unwrap().count(), 0);
}

#[test]
fn terminating_errors_remain_failures_with_utf8_diagnostics() {
    let expected = "검증 오류 보존";
    let output = powershell::command("throw $env:CDR_TEST_TEXT")
        .env("CDR_TEST_TEXT", expected)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains(expected));
}
