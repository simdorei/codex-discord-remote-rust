[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$targetThread = $env:CODEX_THREAD_NAME
$targetThread = if ($targetThread) { $targetThread.Trim() } else { '' }
$projectName = $env:CODEX_PROJECT_NAME
$projectName = if ($projectName) { $projectName.Trim() } else { '' }
if (-not $targetThread) { Write-Output 'NO_THREAD_NAME'; exit 2 }

$scriptDir = $env:CODEX_DESKTOP_BRIDGE_SCRIPT_DIR
if (-not $scriptDir -and $PSScriptRoot) { $scriptDir = $PSScriptRoot }
if (-not $scriptDir -and $PSCommandPath) { $scriptDir = Split-Path -Parent $PSCommandPath }
if (-not $scriptDir) { $scriptDir = (Get-Location).Path }
. (Join-Path $scriptDir 'codex_desktop_bridge_sidebar_native.ps1')
. (Join-Path $scriptDir 'codex_desktop_bridge_sidebar_search.ps1')

$handle = Get-CodexWindowHandle
if ($handle -eq [IntPtr]::Zero) { Write-Output 'NO_CODEX_WINDOW'; exit 3 }
[void][Native]::SetForegroundWindow($handle)
Start-Sleep -Milliseconds 180
$win = Find-CodexAutomationWindow $handle
if (-not $win) { Write-Output 'NO_AUTOMATION_WINDOW'; exit 4 }
$windowRect = $win.Current.BoundingRectangle
$all = Get-AllElements $win
$all = Stabilize-Ui $win $all $windowRect
$all = Expand-ProjectSection $win $all $windowRect $projectName

$target = Find-SidebarElementByNameContains $all $windowRect $targetThread
if (-not $target) {
  $searchResult = Find-ThreadWithScroll $win $all $windowRect $targetThread $projectName
  $all = $searchResult.Elements
  $target = $searchResult.Target
}
if (-not $target) {
  $visible = Get-VisibleSidebarNames $all $windowRect
  if (-not $visible) { $visible = 'NONE' }
  $errors = Get-SidebarActivationErrors
  if ($errors) {
    Write-Output "NOT_FOUND:$targetThread || VISIBLE:$visible || ERRORS:$errors"
  } else {
    Write-Output "NOT_FOUND:$targetThread || VISIBLE:$visible"
  }
  exit 6
}

$buttonCondition = New-Object System.Windows.Automation.PropertyCondition(
  [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
  [System.Windows.Automation.ControlType]::Button
)
$clickTarget = $target.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
if (-not $clickTarget) { $clickTarget = $target }

if (-not (Invoke-Or-Click $clickTarget)) {
  $errors = Get-SidebarActivationErrors
  if ($errors) {
    Write-Output "ACTIVATE_FAILED:$targetThread || ERRORS:$errors"
  } else {
    Write-Output "ACTIVATE_FAILED:$targetThread"
  }
  exit 7
}

Start-Sleep -Milliseconds 800
Write-Output ("OK:" + (Normalize-Name $target.Current.Name))
