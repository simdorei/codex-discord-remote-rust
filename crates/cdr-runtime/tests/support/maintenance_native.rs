use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn run_fixture(root: &Path, directory: &Path, case: &str, variant: &str) {
    run_fixture_with(root, directory, case, variant, |_| {});
}

pub fn run_fixture_with(
    root: &Path,
    directory: &Path,
    case: &str,
    variant: &str,
    configure: impl FnOnce(&mut Command),
) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut command = Command::new("powershell.exe");
    configure(&mut command);
    let mut child=command.args(["-NoProfile","-Command",r"
$ErrorActionPreference='Stop'
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOAD.ps1'),[Text.Encoding]::UTF8)))
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR ('cases/'+$env:CDR_CASE)),[Text.Encoding]::UTF8))) -Variant $env:CDR_VARIANT
exit 0
"]).env("V2_ROOT",root).env("V2_SOURCE",repo).env("CDR_FIXTURE_DIR",directory)
.env("CDR_CASE",case).env("CDR_VARIANT",variant).current_dir(root)
.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("maintenance fixture timed out: {case}/{variant}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{case}/{variant}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
