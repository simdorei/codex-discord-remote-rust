[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$targetThread = $env:CODEX_THREAD_NAME
$targetThread = if ($targetThread) { $targetThread.Trim() } else { '' }
$projectName = $env:CODEX_PROJECT_NAME
$projectName = if ($projectName) { $projectName.Trim() } else { '' }
if (-not $targetThread) { Write-Output 'NO_THREAD_NAME'; exit 2 }

$code = @'
using System;
using System.Runtime.InteropServices;
public static class Native {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int maxCount);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type $code

function Get-CodexWindowHandle {
  $script:result = [IntPtr]::Zero
  $cb = [Native+EnumWindowsProc]{
param($hWnd, $lParam)
if (-not [Native]::IsWindowVisible($hWnd)) { return $true }
$sb = New-Object System.Text.StringBuilder 512
[void][Native]::GetWindowText($hWnd, $sb, $sb.Capacity)
if ($sb.ToString() -like '*Codex*') { $script:result = $hWnd; return $false }
return $true
  }
  [void][Native]::EnumWindows($cb, [IntPtr]::Zero)
  return $script:result
}

function Find-CodexAutomationWindow {
  param([IntPtr]$Handle)
  $cond = New-Object System.Windows.Automation.PropertyCondition(
[System.Windows.Automation.AutomationElement]::NativeWindowHandleProperty,
[int]$Handle
  )
  return [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
[System.Windows.Automation.TreeScope]::Descendants,
$cond
  )
}

function Get-AllElements {
  param($Root)
  return $Root.FindAll(
[System.Windows.Automation.TreeScope]::Descendants,
[System.Windows.Automation.Condition]::TrueCondition
  )
}

function Invoke-Or-Click {
  param($Element)

  $pattern = $null
  if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
try { $pattern.Select(); return $true } catch {}
  }
  $pattern = $null
  if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
try { $pattern.Invoke(); return $true } catch {}
  }
  try {
$point = $Element.GetClickablePoint()
[void][Native]::SetCursorPos([int]$point.X, [int]$point.Y)
Start-Sleep -Milliseconds 80
[Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
return $true
  } catch {}
  try {
$rect = $Element.Current.BoundingRectangle
if ($rect.Width -gt 1 -and $rect.Height -gt 1) {
  $x = [int]($rect.Left + ($rect.Width / 2))
  $y = [int]($rect.Top + ($rect.Height / 2))
  [void][Native]::SetCursorPos($x, $y)
  Start-Sleep -Milliseconds 80
  [Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
  [Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
  return $true
}
  } catch {}
  return $false
}

function Find-ElementByNameContains {
  param($Elements, [string]$ControlTypeName, [string]$Needle)
  foreach ($el in $Elements) {
$name = ($el.Current.Name -replace "`r|`n", ' ').Trim()
$type = $el.Current.ControlType.ProgrammaticName
if ($name -and $type -eq $ControlTypeName -and $name.Contains($Needle)) {
  return $el
}
  }
  return $null
}

function Get-ListItemNames {
  param($Elements)
  $names = New-Object System.Collections.Generic.List[string]
  foreach ($el in $Elements) {
$type = $el.Current.ControlType.ProgrammaticName
if ($type -ne 'ControlType.ListItem') { continue }
$name = ($el.Current.Name -replace "`r|`n", ' ').Trim()
if (-not $name) { continue }
if (-not $names.Contains($name)) { [void]$names.Add($name) }
  }
  return ($names -join ' | ')
}

function Refresh-AllElements {
  param($Window)
  return Get-AllElements $Window
}

function Send-CtrlB {
  [Native]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
  [Native]::keybd_event(0x42, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 50
  [Native]::keybd_event(0x42, 0, 0x0002, [UIntPtr]::Zero)
  [Native]::keybd_event(0x11, 0, 0x0002, [UIntPtr]::Zero)
}

function Expand-ProjectSection {
  param($Window, $Elements, [string]$ProjectName)
  if (-not $ProjectName) { return $Elements }

  $projectItem = Find-ElementByNameContains $Elements 'ControlType.ListItem' $ProjectName
  $projectButton = Find-ElementByNameContains $Elements 'ControlType.Button' $ProjectName
  $expandButton = $null

  foreach ($candidate in @($projectItem, $projectButton)) {
if (-not $candidate) { continue }
$buttonCondition = New-Object System.Windows.Automation.PropertyCondition(
  [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
  [System.Windows.Automation.ControlType]::Button
)
$buttons = $candidate.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
for ($i=0; $i -lt $buttons.Count; $i++) {
  $btn = $buttons.Item($i)
  $btnName = ($btn.Current.Name -replace "`r|`n", ' ').Trim()
  if ($btnName -like '*Expand folder*') { $expandButton = $btn; break }
}
if ($expandButton) { break }
  }

  if ($expandButton) {
if (Invoke-Or-Click $expandButton) {
  Start-Sleep -Milliseconds 300
  return Refresh-AllElements $Window
}
  }

  if ($projectButton) {
if (Invoke-Or-Click $projectButton) {
  Start-Sleep -Milliseconds 300
  return Refresh-AllElements $Window
}
  }
  return $Elements
}

$handle = Get-CodexWindowHandle
if ($handle -eq [IntPtr]::Zero) { Write-Output 'NO_CODEX_WINDOW'; exit 3 }
[void][Native]::SetForegroundWindow($handle)
Start-Sleep -Milliseconds 180
$win = Find-CodexAutomationWindow $handle
if (-not $win) { Write-Output 'NO_AUTOMATION_WINDOW'; exit 4 }
$all = Get-AllElements $win

$hideSidebar = Find-ElementByNameContains $all 'ControlType.Button' 'Hide Sidebar'
if (-not $hideSidebar) {
  $showSidebar = Find-ElementByNameContains $all 'ControlType.Button' 'Show Sidebar'
  if ($showSidebar) {
if (-not (Invoke-Or-Click $showSidebar)) { Write-Output 'SIDEBAR_TOGGLE_FAILED'; exit 5 }
Start-Sleep -Milliseconds 250
$all = Get-AllElements $win
  }
}

$target = Find-ElementByNameContains $all 'ControlType.ListItem' $targetThread
if (-not $target) {
  $projectHit = $null
  if ($projectName) {
$projectHit = Find-ElementByNameContains $all 'ControlType.ListItem' $projectName
if (-not $projectHit) {
  $projectHit = Find-ElementByNameContains $all 'ControlType.Button' $projectName
}
  }
  if (-not $projectHit) {
Send-CtrlB
Start-Sleep -Milliseconds 350
$all = Refresh-AllElements $win
  }
  $all = Expand-ProjectSection $win $all $projectName
  $target = Find-ElementByNameContains $all 'ControlType.ListItem' $targetThread
}
if (-not $target) {
  $visible = Get-ListItemNames $all
  Write-Output "NOT_FOUND:$targetThread || VISIBLE:$visible"
  exit 6
}

$clickTarget = $target
$activated = Invoke-Or-Click $clickTarget
if (-not $activated) {
  $buttonCondition = New-Object System.Windows.Automation.PropertyCondition(
[System.Windows.Automation.AutomationElement]::ControlTypeProperty,
[System.Windows.Automation.ControlType]::Button
  )
  $buttonTarget = $target.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
  if ($buttonTarget) {
$clickTarget = $buttonTarget
$activated = Invoke-Or-Click $clickTarget
  }
}

if (-not $activated) {
  Write-Output "ACTIVATE_FAILED:$targetThread"
  exit 7
}

Start-Sleep -Milliseconds 800
Write-Output ("OK:" + (($target.Current.Name -replace "`r|`n", ' ').Trim()))
