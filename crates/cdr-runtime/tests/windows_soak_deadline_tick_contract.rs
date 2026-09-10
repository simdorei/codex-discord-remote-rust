#![cfg(windows)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn runtime_module_path() -> PathBuf {
    repo_root().join("scripts/CodexDiscordSoak.WrapperRuntime.psm1")
}

fn wrapper_path() -> PathBuf {
    repo_root().join("codex-discord-rust-soak.ps1")
}

fn write_runner(path: &Path) {
    fs::write(
        path,
        r#"param([string]$ModulePath,[string]$WrapperPath,[string]$CaseName)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
Import-Module -Name $ModulePath -Force
switch ($CaseName) {
    'boundary' {
        $limitTicks=[long]10 * [TimeSpan]::TicksPerSecond
        $exact=Test-CodexSoakWithinExitDeadline ([TimeSpan]::FromTicks($limitTicks)) 7 3
        $over=Test-CodexSoakWithinExitDeadline ([TimeSpan]::FromTicks($limitTicks + 1)) 7 3
        [ordered]@{ exact=[bool]$exact; over=[bool]$over } | ConvertTo-Json -Compress
    }
    'negative_limit' { $null=Test-CodexSoakWithinExitDeadline ([TimeSpan]::Zero) -1 0; throw 'negative limit was accepted' }
    'negative_grace' { $null=Test-CodexSoakWithinExitDeadline ([TimeSpan]::Zero) 0 -1; throw 'negative grace was accepted' }
    'negative_elapsed' { $null=Test-CodexSoakWithinExitDeadline ([TimeSpan]::FromTicks(-1)) 0 0; throw 'negative elapsed time was accepted' }
    'overflowing_limit' {
        $null=Test-CodexSoakWithinExitDeadline ([TimeSpan]::Zero) ([long]::MaxValue) 1
        throw 'overflowing limit was accepted'
    }
    'ast' {
        $tokens=$null; $errors=$null
        $ast=[Management.Automation.Language.Parser]::ParseFile($WrapperPath,[ref]$tokens,[ref]$errors)
        if ($errors.Count -ne 0) { throw ($errors[0].Message) }
        $commands=@($ast.FindAll({ param($node) $node -is [Management.Automation.Language.CommandAst] },$true))
        $guards=@($commands | Where-Object {
            $_.GetCommandName() -eq 'Test-CodexSoakWithinExitDeadline'
        } | ForEach-Object {
            $ifNode=$_.Parent
            while ($null -ne $ifNode -and -not ($ifNode -is [Management.Automation.Language.IfStatementAst])) {
                $ifNode=$ifNode.Parent }
            if ($null -eq $ifNode) { throw 'deadline command is not guarded' }
            $parent=$ifNode.Parent; $insideWhile=$false
            while ($null -ne $parent) {
                if ($parent -is [Management.Automation.Language.WhileStatementAst]) { $insideWhile=$true; break }
                $parent=$parent.Parent
            }
            [ordered]@{ start=$_.Extent.StartOffset; if_start=$ifNode.Extent.StartOffset; inside_while=$insideWhile
                direct_while=($ifNode.Parent.Parent -is [Management.Automation.Language.WhileStatementAst]); if_text=$ifNode.Extent.Text }
        })
        $assignments=@($ast.FindAll({ param($node) $node -is
            [Management.Automation.Language.AssignmentStatementAst] -and
            $node.Left.VariablePath.UserPath -match '(^|:)(runClock|runElapsedSeconds|pollMilliseconds)$' },$true))
        $runMethods=@($ast.FindAll({ param($node) $node -is
            [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $node.Expression.VariablePath.UserPath -match '(^|:)runClock$' },$true) |
            ForEach-Object { [string]$_.Member.Value })
        $waits=@($ast.FindAll({ param($node) $node -is
            [Management.Automation.Language.InvokeMemberExpressionAst] -and
            $node.Expression.Extent.Text -eq '$child' -and
            $node.Member.Value -eq 'WaitForExit' },$true) | ForEach-Object {
                [ordered]@{ start=$_.Extent.StartOffset; end=$_.Extent.EndOffset; text=$_.Extent.Text } })
        $protectedNames=@('ConvertFrom-Json','Get-CodexSoakJsonLineCount','Get-CodexSoakMemoryRegression',
            'Get-CodexSoakEligibilityChecks','New-CodexSoakSummaryV3','Publish-CodexSoakSummaryV3','Write-Output')
        $protected=@($commands | Where-Object { $protectedNames -contains $_.GetCommandName() } |
            ForEach-Object { $_.Extent.StartOffset })
        $protected+=@($ast.FindAll({ param($node) $node -is
            [Management.Automation.Language.MemberExpressionAst] -and
            $node.Member.Value -eq 'ExitCode' },$true) |
            ForEach-Object { $_.Extent.StartOffset })
        $earlyExitCount=@($ast.FindAll({ param($node) $node -is
            [Management.Automation.Language.ContinueStatementAst] -or $node -is
            [Management.Automation.Language.BreakStatementAst] -or $node -is
            [Management.Automation.Language.ReturnStatementAst] -or $node -is [Management.Automation.Language.ExitStatementAst] },$true)).Count
        $sleepName='Start' + '-Sleep'
        [ordered]@{ guards=$guards; sleep_count=@($commands | Where-Object { $_.GetCommandName() -eq $sleepName }).Count
            assignments=(@($assignments | ForEach-Object { $_.Extent.Text }) -join '|'); run_methods=$runMethods
            waits=$waits; protected_min=($protected | Measure-Object -Minimum).Minimum; early_exit_count=$earlyExitCount
        } | ConvertTo-Json -Depth 4 -Compress
    }
    default { throw "unknown case: $CaseName" }
}
"#,
    )
    .unwrap();
}

fn invoke(case_name: &str) -> Output {
    let temp = tempfile::tempdir().unwrap();
    let runner = temp.path().join("invoke-deadline.ps1");
    write_runner(&runner);
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(runner)
        .arg("-ModulePath")
        .arg(runtime_module_path())
        .arg("-WrapperPath")
        .arg(wrapper_path())
        .arg("-CaseName")
        .arg(case_name)
        .output()
        .unwrap()
}

#[test]
fn exact_deadline_is_accepted_and_one_timespan_tick_later_is_rejected() {
    let output = invoke("boundary");
    assert!(
        output.status.success(),
        "PowerShell boundary probe failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["exact"], true);
    assert_eq!(result["over"], false);
}

#[test]
fn invalid_or_overflowing_deadline_limits_fail_with_an_error() {
    for case_name in [
        "negative_limit",
        "negative_grace",
        "negative_elapsed",
        "overflowing_limit",
    ] {
        let output = invoke(case_name);
        assert!(
            !output.status.success(),
            "{case_name} unexpectedly succeeded: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("exit deadline"),
            "{case_name} did not surface the deadline error: {error}"
        );
    }
}

#[test]
fn both_guards_use_the_same_tick_helper_before_any_success_evidence() {
    let wrapper_bytes = fs::read(wrapper_path()).unwrap();
    let module_bytes = fs::read(runtime_module_path()).unwrap();
    assert!(!wrapper_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    assert!(!module_bytes.starts_with(&[0xef, 0xbb, 0xbf]));
    let wrapper = String::from_utf8(wrapper_bytes).unwrap();
    let module = String::from_utf8(module_bytes).unwrap();
    assert!(wrapper.lines().count() <= 250);
    assert!(module.lines().count() <= 250);

    assert!(module.contains("function Test-CodexSoakWithinExitDeadline"));
    assert!(module.contains("$Elapsed.Ticks"));
    assert!(module.contains("'Test-CodexSoakWithinExitDeadline'"));

    let clock_start = wrapper
        .find("$runClock = [Diagnostics.Stopwatch]::StartNew()")
        .unwrap();
    let child_start = wrapper.find("if (-not $child.Start())").unwrap();
    let loop_start = wrapper
        .find("while (-not $child.WaitForExit($pollMilliseconds))")
        .unwrap();
    let ast_output = invoke("ast");
    assert!(
        ast_output.status.success(),
        "PowerShell AST probe failed: {}",
        String::from_utf8_lossy(&ast_output.stderr)
    );
    let ast: Value = serde_json::from_slice(&ast_output.stdout).unwrap();
    let guards = ast["guards"].as_array().unwrap();
    assert_eq!(guards.len(), 2);
    assert_eq!(ast["sleep_count"], 0);
    assert_eq!(ast["early_exit_count"], 0);
    assert_eq!(
        ast["assignments"],
        "$runElapsedSeconds = 0.0|$runClock = [Diagnostics.Stopwatch]::StartNew()|$pollMilliseconds = [int][math]::Min(1000, [math]::Ceiling($effectiveInterval * 1000.0))|$runElapsedSeconds = $runClock.Elapsed.TotalSeconds"
    );
    assert_eq!(ast["run_methods"], serde_json::json!(["Stop"]));
    assert_eq!(guards[0]["inside_while"], true);
    assert_eq!(guards[0]["direct_while"], true);
    assert_eq!(guards[1]["inside_while"], false);
    let expected_guard = "if(-not(Test-CodexSoakWithinExitDeadline$runClock.Elapsed$DurationSeconds$HarnessExitGraceSeconds)){throw'Offlinesoakharnessexceededitsexitdeadline'}";
    for guard in guards {
        let actual = guard["if_text"].as_str().unwrap();
        assert_eq!(
            actual.split_whitespace().collect::<String>(),
            expected_guard
        );
    }
    let first_guard = usize::try_from(guards[0]["start"].as_u64().unwrap()).unwrap();
    let sampling = wrapper
        .find("if ($sampleClock.Elapsed.TotalSeconds -ge $effectiveInterval)")
        .unwrap();
    assert!(clock_start < child_start);
    assert!(child_start < loop_start && loop_start < first_guard);
    assert!(first_guard < sampling);

    let waits = ast["waits"].as_array().unwrap();
    assert_eq!(waits.len(), 2);
    assert_eq!(waits[0]["text"], "$child.WaitForExit($pollMilliseconds)");
    assert_eq!(waits[1]["text"], "$child.WaitForExit()");
    let final_wait = usize::try_from(waits[1]["start"].as_u64().unwrap()).unwrap();
    let final_wait_end = usize::try_from(waits[1]["end"].as_u64().unwrap()).unwrap();
    let post_if_start = usize::try_from(guards[1]["if_start"].as_u64().unwrap()).unwrap();
    assert_eq!(
        wrapper[final_wait_end..post_if_start]
            .split_whitespace()
            .collect::<String>(),
        ";$runClock.Stop();$runElapsedSeconds=$runClock.Elapsed.TotalSeconds"
    );
    let clock_stop = wrapper.find("$runClock.Stop();").unwrap();
    assert_eq!(wrapper.matches("$runClock.Stop()").count(), 1);
    let second_guard = usize::try_from(guards[1]["start"].as_u64().unwrap()).unwrap();
    let exit_code = wrapper.find("$exitCode = $child.ExitCode").unwrap();
    let evidence_parse = wrapper.find("$stage = 'parse'").unwrap();
    assert!(final_wait < clock_stop && clock_stop < second_guard);
    assert!(second_guard < exit_code && exit_code < evidence_parse);
    assert!(second_guard < usize::try_from(ast["protected_min"].as_u64().unwrap()).unwrap());

    let helper_start = module
        .find("function Test-CodexSoakWithinExitDeadline")
        .unwrap();
    let helper_end = module[helper_start..]
        .find("\nfunction ")
        .map_or(module.len(), |offset| helper_start + offset);
    let helper_source = &module[helper_start..helper_end];
    assert!(!helper_source.contains("TotalSeconds"));
    let contract_source = include_str!("windows_soak_deadline_tick_contract.rs");
    let thread_sleep = ["thread", "::", "sleep"].concat();
    let tokio_sleep = ["tokio", "::", "time", "::", "sleep"].concat();
    assert!(!contract_source.contains(&thread_sleep));
    assert!(!contract_source.contains(&tokio_sleep));
}
