$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'CdrPackagedDesktop.ps1')
function Assert($Condition,[string]$Message) { if (-not $Condition) { throw $Message } }
# Compile and read this test process's identity, without activating any app.
Initialize-CdrPackagedDesktopInterop
$self=Microsoft.PowerShell.Management\Get-Process -Id $PID
try { $null=Get-CdrProcessPackageFullName $self } finally { $self.Dispose() }

$script:registration=[pscustomobject]@{DesktopPath='C:\fixture\ChatGPT.exe'
    DesktopPackageFullName='OpenAI.Codex_1.0.0.0_x64__publisher'
    DesktopAppUserModelId='OpenAI.Codex_publisher!App'}
$script:activations=0
function Get-CdrPackagedDesktopRegistration { return $script:registration }
function Get-CdrProcessPackageFullName($Process) { return $Process.Package }
function Get-Process {
    param($Id)
    if (-not $script:processes.ContainsKey([int]$Id)) { return $null }
    # Each call owns a distinct handle wrapper, as System.Diagnostics.Process does.
    $copy=$script:processes[[int]$Id] | Select-Object *
    $copy | Add-Member ScriptMethod Dispose { $this.Disposed=$true }
    $script:handles.Add($copy)
    return $copy
}
function Get-CimInstance {
    param($ClassName,$Filter)
    Assert ($ClassName -eq 'Win32_Process') 'Unexpected process query.'
    if ($Filter -eq "Name='ChatGPT.exe'") { return $script:entries }
    if ($Filter -match "^ParentProcessId=(\d+) AND Name='codex.exe'$") {
        $script:backendProbes++
        if ($script:backendProbes -le $script:backendDelay) { return @() }
        if ($script:backendParents -contains [int]$Matches[1]) {
            return [pscustomobject]@{CommandLine='codex.exe app-server --stdio'}
        }
        return @()
    }
    throw "Unexpected filter: $Filter"
}
function Invoke-CdrRegisteredDesktopActivation {
    param($AppUserModelId)
    Assert ($AppUserModelId -ceq 'OpenAI.Codex_publisher!App') 'The registered app id was not used.'
    $script:activations++; return 42
}
function Add-FixtureProcess([int]$Id,[int]$ParentId,[DateTime]$Started) {
    $script:processes[$Id]=[pscustomobject]@{Id=$Id;Handle=$Id;Path=$script:registration.DesktopPath
        HasExited=$false;Disposed=$false;StartTime=$Started;Package=$script:registration.DesktopPackageFullName}
    $script:entries+= [pscustomobject]@{ProcessId=$Id;ParentProcessId=$ParentId;CreationDate=$Started
        ExecutablePath=$script:registration.DesktopPath;CommandLine='ChatGPT.exe'}
}
function Reset-Fixture {
    $script:handles=[Collections.Generic.List[object]]::new()
    $script:processes=@{};$script:entries=@();$script:backendParents=@(42)
    $script:backendProbes=0;$script:backendDelay=0
    $script:created=[DateTime]::UtcNow
    Add-FixtureProcess 42 10 $script:created
}
function Assert-Refused([string]$Message) {
    $failure=$null
    try { $null=Start-CdrPackagedDesktop $plan -WaitBudget ([TimeSpan]::Zero) } catch { $failure=$_.Exception.Message }
    Assert (-not [string]::IsNullOrEmpty($failure)) $Message
    Assert ($failure -match 'did not match its registered package|Desktop relaunch process identity changed|Multiple activated desktop hosts|did not produce a verified packaged desktop') "Unexpected refusal: $failure"
    Assert (@($script:handles | Where-Object { -not $_.Disposed }).Count -eq 0) 'Failed resolution leaked a handle.'
}
$plan=$script:registration | ConvertTo-Json | ConvertFrom-Json
Reset-Fixture
$started=Start-CdrPackagedDesktop $plan -WaitBudget ([TimeSpan]::Zero)
Assert ($started.Id -eq 42 -and -not $started.Disposed) 'Verified process handle was not retained.'
Assert $script:handles[0].Disposed 'Activation handle was not released.'
$started.Dispose()

Reset-Fixture
$script:backendDelay=1
$started=Start-CdrPackagedDesktop $plan -WaitBudget ([TimeSpan]::FromSeconds(2))
Assert ($started.Id -eq 42 -and $script:backendProbes -ge 2) 'Backend readiness was not awaited.'
$started.Dispose()

foreach ($field in @('DesktopPath','DesktopPackageFullName','DesktopAppUserModelId')) {
    $stale=$plan | ConvertTo-Json | ConvertFrom-Json
    $stale.$field='changed'
    $before=$script:activations;$refused=$false
    try { $null=Start-CdrPackagedDesktop $stale -WaitBudget ([TimeSpan]::Zero) } catch { $refused=$true }
    Assert ($refused -and $script:activations -eq $before) "Stale $field activated an app."
}
foreach ($identity in @($null,'Other.Package_1.0.0.0_x64__publisher')) {
    Reset-Fixture
    $script:processes[42].Package=$identity
    Assert-Refused 'Wrong/unpackaged activation process was accepted.'
}
Reset-Fixture
$script:processes[42].Path='C:\other\ChatGPT.exe'
Assert-Refused 'Wrong activation executable with a matching package was accepted.'

Reset-Fixture
$script:processes[42].HasExited=$true
Assert-Refused 'Exited activation process without a relaunch was accepted.'

# Electron may replace the activation process before its backend is ready.
Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 43 42 ($script:created.AddMilliseconds(20))
$script:backendParents=@(43)
$started=Start-CdrPackagedDesktop $plan -WaitBudget ([TimeSpan]::Zero)
Assert ($started.Id -eq 43 -and -not $started.Disposed) 'Verified direct relaunch was not retained.'
Assert $script:handles[0].Disposed 'Original activation handle was not released after relaunch.'
$started.Dispose()

Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 44 11 ($script:created.AddSeconds(-10))
$script:backendParents=@(44)
Assert-Refused 'An unrelated pre-existing packaged desktop was adopted.'

Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 43 42 ($script:created.AddSeconds(-10))
$script:backendParents=@(43)
Assert-Refused 'A child predating the activation process was adopted.'

Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 43 42 ($script:created.AddMilliseconds(20))
$script:backendParents=@(43)
$script:processes[43].StartTime=$script:created.AddSeconds(1)
Assert-Refused 'A reused relaunch process id was accepted.'

Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 43 42 ($script:created.AddMilliseconds(20))
$script:backendParents=@(43)
$script:processes[43].Package=$null
Assert-Refused 'An unpackaged relaunch child was accepted.'

Reset-Fixture
Add-FixtureProcess 43 42 ($script:created.AddMilliseconds(20))
$script:backendParents=@(42,43)
Assert-Refused 'Multiple candidate desktop hosts were accepted.'

Reset-Fixture
$script:backendParents=@()
Assert-Refused 'A bootstrap process without a backend was reported ready.'

Reset-Fixture
$script:processes[42].HasExited=$true
Add-FixtureProcess 43 42 ($script:created.AddMilliseconds(20))
$script:entries[1].CommandLine='ChatGPT.exe --type=renderer'
$script:backendParents=@(43)
Assert-Refused 'An Electron renderer was treated as a desktop host.'
Write-Output 'packaged_desktop_tests_passed'
