#![cfg(windows)]
use std::{fs, path::Path, process::Command};

#[test]
fn helper_verification_failure_preserves_the_original_plugin_binary() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("build/cdr-pro-helper.exe");
    let target = root
        .path()
        .join("plugins/codex-discord-remote/bin/cdr-pro-helper.exe");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&source, b"new helper").unwrap();
    fs::write(&target, b"known good helper").unwrap();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let module = root.path().join("CdrInstallPluginHelper.psm1");
    let injected = fs::read_to_string(scripts.join("CdrInstallPluginHelper.psm1"))
        .unwrap()
        .replace("Get-CdrInstallArtifactHash", "Get-CdrTestArtifactHash");
    fs::write(&module, format!("{injected}\nfunction Get-CdrTestArtifactHash {{ param([string]$Path) if ([IO.Path]::GetFullPath($Path) -eq [IO.Path]::GetFullPath($env:CDR_TEST_SOURCE)) {{ return 'expected' }} return 'changed during copy' }}\n")).unwrap();
    fs::copy(
        scripts.join("CdrInstallRuntime.psm1"),
        root.path().join("CdrInstallRuntime.psm1"),
    )
    .unwrap();
    let script = r"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_TEST_MODULE -Force
try {
  Install-CdrPluginHelper -RepoRoot $env:CDR_TEST_ROOT -RuntimeBinary (Join-Path (Split-Path $env:CDR_TEST_SOURCE -Parent) 'cdr-runtime.exe') -SkipBuild
  throw 'verification failure was not reported'
} catch {
  if ($_.Exception.Message -notmatch 'does not match') { throw }
  Write-Output 'verification refused'
}
";
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("CDR_TEST_ROOT", root.path())
        .env("CDR_TEST_SOURCE", &source)
        .env("CDR_TEST_MODULE", module)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("verification refused"));
    assert_eq!(fs::read(&target).unwrap(), b"known good helper");
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn valid_helper_is_published_and_same_artifact_staging_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("build/cdr-pro-helper.exe");
    let target = root
        .path()
        .join("plugins/codex-discord-remote/bin/cdr-pro-helper.exe");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&source, b"new helper").unwrap();
    fs::write(&target, b"known good helper").unwrap();
    let module =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/CdrInstallPluginHelper.psm1");
    let script = r"
$ErrorActionPreference = 'Stop'
Import-Module $env:CDR_TEST_MODULE -Force
Install-CdrPluginHelper -RepoRoot $env:CDR_TEST_ROOT -RuntimeBinary (Join-Path (Split-Path $env:CDR_TEST_SOURCE -Parent) 'cdr-runtime.exe') -SkipBuild
Install-CdrPluginHelper -RepoRoot $env:CDR_TEST_ROOT -RuntimeBinary $env:CDR_TEST_INSTALLED -SkipBuild
";
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", script])
        .env("CDR_TEST_ROOT", root.path())
        .env("CDR_TEST_SOURCE", &source)
        .env(
            "CDR_TEST_INSTALLED",
            target.parent().unwrap().join("cdr-runtime.exe"),
        )
        .env("CDR_TEST_MODULE", module)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"new helper");
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
}
