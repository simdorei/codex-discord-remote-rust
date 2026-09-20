param(
    [Parameter(Mandatory=$true)][string]$SourceRoot,
    [Parameter(Mandatory=$true)][string]$TemporaryRoot
)
$ErrorActionPreference = 'Stop'
$source = [IO.Path]::GetFullPath($SourceRoot)
$temporary = [IO.Path]::GetFullPath($TemporaryRoot)
if (-not [IO.Directory]::Exists($source) -or -not [IO.Directory]::Exists($temporary)) {
    throw 'Both diagnostic roots must already exist'
}
$runRoot = [IO.Path]::Combine($temporary, 'cdr-load-init-' + [Guid]::NewGuid().ToString('N'))
if (-not $runRoot.StartsWith($temporary.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Diagnostic root must remain within the temporary directory'
}
[void][IO.Directory]::CreateDirectory($runRoot)
$fixtureSource = [IO.Path]::Combine($source, 'crates\cdr-runtime\..\..')
$fixtureDir = [IO.Path]::Combine($fixtureSource, 'crates\cdr-runtime\tests\fixtures\maintenance')
$loadPath = [IO.Path]::Combine($fixtureDir, 'LOAD.ps1')
$digest = [Security.Cryptography.SHA256]::Create()
try { $loadHash = [BitConverter]::ToString($digest.ComputeHash([IO.File]::ReadAllBytes($loadPath))).Replace('-','').ToLowerInvariant() }
finally { $digest.Dispose() }
Write-Output ('LOAD_SHA256=' + $loadHash)

# Run the exact original parent Join-Path expression before instrumenting complete
# top-level LOAD statements. No case or operational maintenance action is invoked.
$childText = @'
$ErrorActionPreference='Stop'
function Write-ProbeStage([string]$Label) {
    [IO.File]::AppendAllText($env:CDR_DIAGNOSTIC_TRACE,
        [DateTimeOffset]::UtcNow.ToString('o')+' '+$Label+[Environment]::NewLine)
}
try {
    Write-ProbeStage 'child_start'
    if ($env:CDR_DIAGNOSTIC_VARIANT -eq 'explicit_modules') {
        Write-ProbeStage 'management_import_start'
        Import-Module ([IO.Path]::Combine($PSHOME,'Modules\Microsoft.PowerShell.Management\Microsoft.PowerShell.Management.psd1')) -ErrorAction Stop
        Write-ProbeStage 'management_import_done'
        Import-Module ([IO.Path]::Combine($PSHOME,'Modules\Microsoft.PowerShell.Utility\Microsoft.PowerShell.Utility.psd1')) -ErrorAction Stop
        Write-ProbeStage 'utility_import_done'
    }
    Write-ProbeStage 'parent_join_start'
    $probeLoadPath = Join-Path $env:CDR_FIXTURE_DIR 'LOAD.ps1'
    Write-ProbeStage 'parent_join_done'
    $probeLoadText = [IO.File]::ReadAllText($probeLoadPath,[Text.Encoding]::UTF8)
    Write-ProbeStage 'read_done'
    $probeTokens=$null; $probeErrors=$null
    $probeAst=[Management.Automation.Language.Parser]::ParseInput($probeLoadText,[ref]$probeTokens,[ref]$probeErrors)
    if ($probeErrors.Count -ne 0) { throw 'LOAD parsing failed' }
    Write-ProbeStage 'parse_done'
    $probeIndex=0
    foreach ($probeStatement in $probeAst.EndBlock.Statements) {
        $probeLabel='statement_'+$probeIndex.ToString('D2')+'_line_'+$probeStatement.Extent.StartLineNumber
        Write-ProbeStage ($probeLabel+'_start')
        . ([scriptblock]::Create($probeStatement.Extent.Text))
        Write-ProbeStage ($probeLabel+'_done')
        $probeIndex++
    }
    Write-ProbeStage 'load_done'
    exit 0
} catch {
    Write-ProbeStage 'child_error'
    throw
}
'@
$encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($childText))
$variants = @('baseline_1','explicit_modules','baseline_2','os_directories','os_and_module_path','baseline_3')
$baseNames = @('PATH','SYSTEMROOT','WINDIR','TEMP','TMP','COMSPEC','PATHEXT')
$extraNames = @('APPDATA','LOCALAPPDATA','ProgramData','ProgramFiles','ProgramFiles(x86)','ProgramW6432','PSModuleAnalysisCachePath')
$results = [Collections.Generic.List[object]]::new()
foreach ($variant in $variants) {
    $root = [IO.Path]::Combine($runRoot,$variant)
    [void][IO.Directory]::CreateDirectory($root)
    $trace = [IO.Path]::Combine($root,'stages.txt')
    [IO.File]::WriteAllText([IO.Path]::Combine($root,'codex.exe'),'not executable; diagnostic must not launch an app-server')
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = [IO.Path]::Combine($env:SYSTEMROOT,'System32\WindowsPowerShell\v1.0\powershell.exe')
    $start.Arguments = '-NoProfile -EncodedCommand ' + $encoded
    $start.WorkingDirectory = $root
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.EnvironmentVariables.Clear()
    $names = $baseNames
    if ($variant -in @('os_directories','os_and_module_path')) { $names += $extraNames }
    if ($variant -eq 'os_and_module_path') { $names += 'PSModulePath' }
    foreach ($name in $names) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if ($null -ne $value) { $start.EnvironmentVariables[$name]=$value }
    }
    # Equivalent to the reviewed workflow's checked TEMP/TMP setup.
    $start.EnvironmentVariables['TEMP']=$temporary
    $start.EnvironmentVariables['TMP']=$temporary
    $start.EnvironmentVariables['USERPROFILE']=$root
    $start.EnvironmentVariables['CODEX_HOME']=[IO.Path]::Combine($root,'codex-home')
    $start.EnvironmentVariables['CODEX_EXE']=[IO.Path]::Combine($root,'codex.exe')
    $start.EnvironmentVariables['CODEX_DISCORD_ROOT']=$root
    $start.EnvironmentVariables['CODEX_DISCORD_MIRROR_DB']=[IO.Path]::Combine($root,'discord_mirror.sqlite')
    $start.EnvironmentVariables['CODEX_STATE_DB']=[IO.Path]::Combine($root,'unused.sqlite')
    $start.EnvironmentVariables['V2_ROOT']=$root
    $start.EnvironmentVariables['V2_SOURCE']=$fixtureSource
    $start.EnvironmentVariables['CDR_FIXTURE_DIR']=$fixtureDir
    $start.EnvironmentVariables['CDR_CASE']='real_snapshot_receipts.ps1'
    $start.EnvironmentVariables['CDR_VARIANT']=''
    $start.EnvironmentVariables['CDR_DIAGNOSTIC_TRACE']=$trace
    $start.EnvironmentVariables['CDR_DIAGNOSTIC_VARIANT']=$variant
    $child = [Diagnostics.Process]::new()
    $child.StartInfo=$start
    $childStarted=$false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        if (-not $child.Start()) { throw 'Could not start owned diagnostic child' }
        $childStarted=$true
        $stdoutTask = $child.StandardOutput.ReadToEndAsync()
        $stderrTask = $child.StandardError.ReadToEndAsync()
        $completed = $child.WaitForExit(35000)
        if (-not $completed) { $child.Kill() }
        if (-not $child.WaitForExit(5000)) { throw 'Owned diagnostic child did not exit after cleanup' }
        if (-not $stdoutTask.Wait(5000) -or -not $stderrTask.Wait(5000)) { throw 'Diagnostic streams did not close' }
        $stdout = $stdoutTask.Result
        $stderr = $stderrTask.Result
        $watch.Stop()
        $result = [ordered]@{ variant=$variant; timeout=(-not $completed); exit_code=$child.ExitCode; elapsed_ms=$watch.ElapsedMilliseconds; stdout_characters=$stdout.Length; stderr_characters=$stderr.Length }
        $results.Add([pscustomobject]$result)
        Write-Output ('RESULT ' + ($result | ConvertTo-Json -Compress))
        if ([IO.File]::Exists($trace)) {
            $stream=[IO.File]::OpenRead($trace)
            try { $buffer=[byte[]]::new(16384); $count=$stream.Read($buffer,0,$buffer.Length); Write-Output ([Text.Encoding]::UTF8.GetString($buffer,0,$count)) }
            finally { $stream.Dispose() }
        }
        if ($stdout.Length -gt 0) { Write-Output ('STDOUT ' + $stdout.Substring(0,[Math]::Min(4096,$stdout.Length))) }
        if ($stderr.Length -gt 0) { Write-Output ('STDERR ' + $stderr.Substring(0,[Math]::Min(4096,$stderr.Length))) }
    } finally {
        if ($childStarted -and -not $child.HasExited) { $child.Kill(); [void]$child.WaitForExit(5000) }
        $child.Dispose()
    }
}
[IO.File]::WriteAllText([IO.Path]::Combine($runRoot,'results.json'),($results | ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
Write-Output 'DIAGNOSTIC_ONLY_COMPLETE_NOT_A_CONTRACT_TEST_PASS'
