[CmdletBinding()]
param([string]$Repository,[string]$FixtureRoot,[string]$Candidate,[string]$Scenario)
$ErrorActionPreference='Stop'
$Root=[IO.Path]::GetFullPath($FixtureRoot)
$Candidate=[IO.Path]::GetFullPath($Candidate)
$environment=Join-Path $Root '.env'
$database=Join-Path $Root 'store.sqlite'
$capability=Join-Path $Root 'capabilities.json'
$review=Join-Path $Root 'launch-artifacts.json'
$required=Join-Path $Root '.codex_discord_rust.compatibility.required'
$contract=Join-Path $Root '.codex_discord_rust.compatibility.json'
$seal=Join-Path $Root '.codex_discord_bot.disabled'
$operation='fixture-install-v1'
$candidateDirectory=Join-Path $Repository 'scripts/candidates/async-recovery-v1'
. (Join-Path $Repository 'codex-discord-rust-control.ps1')
. (Join-Path $candidateDirectory 'CdrAsyncRecoveryCompatibility.ps1')
. (Join-Path $candidateDirectory 'CdrRuntimeLaunchCompatibility.ps1')
. (Join-Path $candidateDirectory 'CdrInstallLaunchCompatibility.ps1')

function Hash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function TextHash([string]$Text) {
    $sha=[Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash([Text.UTF8Encoding]::new($false).GetBytes($Text))).Replace('-','').ToLowerInvariant() }
    finally { $sha.Dispose() }
}
$files=@(
    @('codex-discord-rust-watchdog.ps1','scripts/candidates/async-recovery-v1/codex-discord-rust-watchdog.ps1'),
    @('codex-discord-rust-control.ps1','codex-discord-rust-control.ps1'),
    @('codex-discord-rust-drain.ps1','codex-discord-rust-drain.ps1'),
    @('scripts/CdrAsyncRecoveryCompatibility.ps1','scripts/candidates/async-recovery-v1/CdrAsyncRecoveryCompatibility.ps1'),
    @('scripts/CdrRuntimeLaunchCompatibility.ps1','scripts/candidates/async-recovery-v1/CdrRuntimeLaunchCompatibility.ps1'),
    @('scripts/CdrDeploymentRecovery.ps1','scripts/candidates/async-recovery-v1/CdrDeploymentRecovery.ps1'),
    @('scripts/CdrLaunchJournal.ps1','scripts/CdrLaunchJournal.ps1'),
    @('scripts/CdrRestartTransaction.ps1','scripts/CdrRestartTransaction.ps1'),
    @('scripts/CdrForceRestart.ps1','scripts/CdrForceRestart.ps1')
)
$entries=@()
foreach ($pair in $files) {
    $destination=Join-Path $Root $pair[0]
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
    Copy-Item -LiteralPath (Join-Path $Repository $pair[1]) -Destination $destination
    $entries+=@{path=$pair[0];sha256=(Hash $destination)}
}
[IO.File]::WriteAllText($environment,"CODEX_DISCORD_MIRROR_DB=$database",[Text.UTF8Encoding]::new($false))
$binaryHash=Hash $Candidate
[IO.File]::WriteAllText($capability,([ordered]@{
    protocol='cdr-artifact-capabilities-v1';artifact_sha256=$binaryHash
    async_resolution_max_format=1;async_recovery_policy_max_format=1
}|ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($review,([ordered]@{
    protocol='cdr-runtime-launch-artifacts-v1';files=$entries
}|ConvertTo-Json -Depth 5 -Compress),[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($seal,$operation,[Text.UTF8Encoding]::new($false))
$parameters=@{
    RepoRoot=$Root;CandidatePath=$Candidate;EnvironmentPath=$environment;DatabasePath=$database
    CapabilityManifestPath=$capability;ExpectedCandidateSha256=$binaryHash
    ExpectedEnvironmentSha256=(Hash $environment);ExpectedCapabilitySha256=(Hash $capability)
    ReviewedLaunchManifestPath=$review;ExpectedLaunchManifestSha256=(Hash $review)
    Operation=$operation;DeadlineUtc=[DateTimeOffset]::UtcNow.AddSeconds(30)
}
$contractText=[ordered]@{
    protocol='cdr-runtime-launch-v1';root=$Root
    candidate_path=$Candidate;candidate_sha256=$binaryHash
    environment_path=$environment;environment_sha256=$parameters.ExpectedEnvironmentSha256
    database_path=$database;capability_path=$capability;capability_sha256=$parameters.ExpectedCapabilitySha256
    launcher_manifest_path=$review;launcher_manifest_sha256=$parameters.ExpectedLaunchManifestSha256
    installed_by=$operation
}|ConvertTo-Json -Compress
$contractHash=TextHash $contractText
if ($Scenario -in @('required-only','foreign')) {
    [IO.File]::WriteAllText($required, $(if($Scenario -eq 'foreign'){'0'*64}else{$contractHash}),[Text.UTF8Encoding]::new($false))
}
if ($Scenario -eq 'contract-only') {
    [IO.File]::WriteAllText($contract,$contractText,[Text.UTF8Encoding]::new($false))
}
if ($Scenario -eq 'foreign-seal') { [IO.File]::WriteAllText($seal,'foreign-owner') }
if ($Scenario -eq 'changed-launcher') {
    [IO.File]::AppendAllText((Join-Path $Root 'codex-discord-rust-watchdog.ps1'),([Environment]::NewLine+'# changed'))
}
if ($Scenario -eq 'changed-env') { [IO.File]::AppendAllText($environment,([Environment]::NewLine+'CHANGED=1')) }
if ($Scenario -eq 'expired') { $parameters.DeadlineUtc=[DateTimeOffset]::UtcNow.AddSeconds(-1) }
$preserve=@{}
foreach ($path in @($required,$contract,$seal)) {
    if ([IO.File]::Exists($path)) { $preserve[$path]=[IO.File]::ReadAllText($path) }
}
$script:actualNewMarker=(Get-Command Write-NewCdrMarker).ScriptBlock
if ($Scenario -eq 'interrupted') {
    function Write-NewCdrMarker([string]$Path,[string]$Text) {
        if ([StringComparer]::OrdinalIgnoreCase.Equals($Path,$contract)) { throw 'fixture_contract_write_interrupted' }
        & $script:actualNewMarker -Path $Path -Text $Text
    }
}
$script:probes=0
$script:actualCompatibility=(Get-Command Assert-CdrAsyncRecoveryCompatibility).ScriptBlock
function Assert-CdrAsyncRecoveryCompatibility {
    param([string]$CandidatePath,[string]$DatabasePath,[string]$CapabilityManifestPath,
        [string]$ExpectedCandidateSha256,[string]$ExpectedCapabilitySha256,
        [int]$TimeoutSeconds=10,[string]$EnvironmentPath,[string]$ExpectedEnvironmentSha256,
        [string]$WorkingDirectory,[switch]$RequireRecoveryPolicy,
        [DateTimeOffset]$DeadlineUtc=[DateTimeOffset]::MaxValue)
    $script:probes++
    & $script:actualCompatibility @PSBoundParameters
}
$script:launches=0
$launch={
    $start=New-Object Diagnostics.ProcessStartInfo
    $start.FileName=$Candidate
    $start.Arguments='--help'
    $start.WorkingDirectory=$Root
    $start.UseShellExecute=$false
    $start.CreateNoWindow=$true
    $start.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput=$true
    $start.RedirectStandardError=$true
    $child=New-Object Diagnostics.Process
    $child.StartInfo=$start
    try {
        if(-not $child.Start()){throw 'native fixture child did not start'}
        $script:launches++
        $stdout=$child.StandardOutput.ReadToEndAsync()
        $stderr=$child.StandardError.ReadToEndAsync()
        if(-not $child.WaitForExit(5000)){throw 'native fixture help child timed out'}
        if($child.ExitCode -ne 0 -or -not $stdout.GetAwaiter().GetResult().Contains('--env')){
            throw ('native fixture help failed: '+$stderr.GetAwaiter().GetResult())
        }
    } finally {
        if($script:launches -gt 0 -and -not $child.HasExited){$child.Kill();[void]$child.WaitForExit(3000)}
        $child.Dispose()
    }
}
$guard=$null
$errorText=''
try {
    if($Scenario -ne 'no-control'){$guard=Enter-CdrControl -Root $Root -Purpose 'maintenance'}
    if($Scenario -eq 'shared-control'){
        $guard.Dispose()
        $guard=[IO.File]::Open((Join-Path $Root '.codex_discord_rust.control.lock'),'Open','ReadWrite','ReadWrite')
    }
    $parameters.ControlGuard=$guard
    $launchParameters=@{RepoRoot=$Root;CandidatePath=$Candidate;EnvironmentPath=$environment
        ControlGuard=$guard;DeadlineUtc=$parameters.DeadlineUtc;Launch=$launch}
    try {
        $result=Install-CdrRuntimeLaunchCompatibility @parameters
        if($result.admission_authorized -ne $false -or $result.recovery_authorized -ne $false -or
            $result.launch_performed -ne $false -or $result.contract_sha256 -cne $contractHash){
            throw 'arming result was not the exact non-authorizing contract'
        }
        if($Scenario -eq 'repeat'){
            $beforeRequired=[IO.File]::ReadAllText($required)
            $beforeContract=[IO.File]::ReadAllText($contract)
            $beforeRequiredTime=[IO.File]::GetLastWriteTimeUtc($required).Ticks
            $beforeContractTime=[IO.File]::GetLastWriteTimeUtc($contract).Ticks
            $null=Install-CdrRuntimeLaunchCompatibility @parameters
            if([IO.File]::ReadAllText($required) -cne $beforeRequired -or
                [IO.File]::ReadAllText($contract) -cne $beforeContract -or
                [IO.File]::GetLastWriteTimeUtc($required).Ticks -ne $beforeRequiredTime -or
                [IO.File]::GetLastWriteTimeUtc($contract).Ticks -ne $beforeContractTime){
                throw 'identical installation rewrote persistent intent'
            }
        }
        if($script:launches -ne 0){throw 'installer launched a child'}
        Invoke-CdrCheckedRuntimeLaunch @launchParameters
    } catch {$errorText=$_.Exception.Message}
    $positive=$Scenario -in @('valid','repeat','required-only','contract-only')
    $expectedLaunches=if($positive){1}else{0}
    $expectedProbes=if($Scenario -eq 'repeat'){3}elseif($positive){2}elseif($Scenario -eq 'interrupted'){1}else{0}
    if($script:launches -ne $expectedLaunches -or $script:probes -ne $expectedProbes){
        throw "wrong counts launches=$script:launches probes=$script:probes error=$errorText"
    }
    if($positive -and $errorText){throw $errorText}
    if(-not $positive -and -not $errorText){throw 'negative installer case had no rejection'}
    foreach($path in $preserve.Keys){
        if([IO.File]::ReadAllText($path) -cne $preserve[$path]){throw 'existing intent or maintenance seal changed'}
    }
    if($Scenario -eq 'interrupted'){
        if(-not [IO.File]::Exists($required) -or [IO.File]::Exists($contract)){throw 'interrupted install did not preserve required-only intent'}
        $partialError=''
        try {Invoke-CdrCheckedRuntimeLaunch @launchParameters}
        catch {$partialError=$_.Exception.Message}
        if(-not $partialError.Contains('Incomplete armed compatibility installation') -or $script:launches -ne 0){
            throw 'partial install did not fence the actual native launch boundary'
        }
    } elseif(-not $positive -and $Scenario -ne 'foreign'){
        if([IO.File]::Exists($required) -or [IO.File]::Exists($contract)){throw 'rejected installation created intent'}
    }
    $expectedError=@{
        'foreign'='Existing required intent differs'
        'no-control'='retained root control lease'
        'shared-control'='exclusive write lease'
        'foreign-seal'='Pinned artifact hash mismatch'
        'changed-launcher'='Pinned artifact hash mismatch'
        'changed-env'='Pinned artifact hash mismatch'
        'expired'='installation_deadline_exceeded'
        'interrupted'='fixture_contract_write_interrupted'
    }
    if($expectedError.ContainsKey($Scenario) -and -not $errorText.Contains($expectedError[$Scenario])){
        throw "wrong rejection boundary: $errorText"
    }
    [Console]::WriteLine((@{scenario=$Scenario;launches=$script:launches;probes=$script:probes
        error=$errorText;operating_actions=$false}|ConvertTo-Json -Compress))
} finally {if($null -ne $guard){$guard.Dispose()}}
