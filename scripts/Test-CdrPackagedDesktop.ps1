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
$script:observedPackage=$script:registration.DesktopPackageFullName
$script:activations=0
$script:process=[pscustomobject]@{Id=42;Handle=1;Path=$script:registration.DesktopPath;HasExited=$false;Disposed=$false}
$script:process | Add-Member ScriptMethod Dispose { $this.Disposed=$true }
function Get-CdrPackagedDesktopRegistration { return $script:registration }
function Get-CdrProcessPackageFullName { return $script:observedPackage }
function Get-Process { param($Id) Assert ($Id -eq 42) 'Wrong activation process id.'; return $script:process }
function Invoke-CdrRegisteredDesktopActivation {
    param($AppUserModelId)
    Assert ($AppUserModelId -ceq 'OpenAI.Codex_publisher!App') 'The registered app id was not used.'
    $script:activations++; return 42
}
$plan=$script:registration | ConvertTo-Json | ConvertFrom-Json
$started=Start-CdrPackagedDesktop $plan
Assert ($started.Id -eq 42 -and -not $started.Disposed) 'Verified process handle was not retained.'

foreach ($field in @('DesktopPath','DesktopPackageFullName','DesktopAppUserModelId')) {
    $stale=$plan | ConvertTo-Json | ConvertFrom-Json
    $stale.$field='changed'
    $before=$script:activations;$refused=$false
    try { $null=Start-CdrPackagedDesktop $stale } catch { $refused=$true }
    Assert ($refused -and $script:activations -eq $before) "Stale $field activated an app."
}
foreach ($identity in @($null,'Other.Package_1.0.0.0_x64__publisher')) {
    $script:observedPackage=$identity;$script:process.Disposed=$false;$refused=$false
    try { $null=Start-CdrPackagedDesktop $plan } catch { $refused=$true }
    Assert ($refused -and $script:process.Disposed) 'Wrong/unpackaged process was accepted.'
}
$script:observedPackage=$plan.DesktopPackageFullName
$script:process.Path='C:\other\ChatGPT.exe';$refused=$false
try { $null=Start-CdrPackagedDesktop $plan } catch { $refused=$true }
Assert $refused 'Wrong executable with a matching package was accepted.'
$script:process.Path=$plan.DesktopPath;$script:process.HasExited=$true;$refused=$false
try { $null=Start-CdrPackagedDesktop $plan } catch { $refused=$true }
Assert $refused 'Exited activation process was accepted.'
Write-Output 'packaged_desktop_tests_passed'
