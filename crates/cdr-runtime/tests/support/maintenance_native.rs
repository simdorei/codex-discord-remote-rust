use std::{
    fs::File,
    io::Read,
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
function Write-CdrFixtureStage([string]$Stage) {
    if ($env:CDR_CASE -cne 'real_snapshot_receipts.ps1') { return }
    $path=[IO.Path]::Combine($env:V2_ROOT,'maintenance-fixture-stages.txt')
    [IO.File]::AppendAllText($path,([DateTimeOffset]::UtcNow.ToString('o')+' '+$Stage+[Environment]::NewLine))
}
Write-CdrFixtureStage 'fixture_start'
. ([scriptblock]::Create([IO.File]::ReadAllText((Join-Path $env:CDR_FIXTURE_DIR 'LOAD.ps1'),[Text.Encoding]::UTF8)))
Write-CdrFixtureStage 'fixture_loaded'
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
            let mut stages = Vec::new();
            if let Ok(file) = File::open(root.join("maintenance-fixture-stages.txt")) {
                let _ = file.take(8192).read_to_end(&mut stages);
            }
            panic!(
                "maintenance fixture timed out: {case}/{variant}\nfixture stages:\n{}",
                String::from_utf8_lossy(&stages)
            );
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
