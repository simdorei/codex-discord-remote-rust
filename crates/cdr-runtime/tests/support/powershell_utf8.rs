use std::process::Command;

/// Test-only PowerShell host with an explicit UTF-8 output contract.
/// Pass paths and other variable data through environment variables, not script text.
pub fn command(script: &str) -> Command {
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
    ]);
    command.arg(format!(
        "$ErrorActionPreference='Stop'; \
         [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); \
         $OutputEncoding=[Console]::OutputEncoding; {script}"
    ));
    command
}
