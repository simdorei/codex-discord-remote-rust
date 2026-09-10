[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$decision = [string]$env:CODEX_APPROVAL_DECISION
$decision = $decision.Trim().ToLowerInvariant()
if (-not $decision) { Write-Output 'NO_DECISION'; exit 2 }

$code = @"
using System;
using System.Runtime.InteropServices;
public static class Native {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int maxCount);
  [DllImport("user32.dll")] public static extern int GetWindowTextLength(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, UIntPtr dwExtraInfo);
}
"@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type $code

function Normalize-Name {
  param([string]$Name)
  if (-not $Name) { return '' }
  return (($Name -replace "`r|`n", ' ') -replace '\s+', ' ').Trim()
}

function Get-CodexWindowHandle {
  $script:result = [IntPtr]::Zero
  $cb = [Native+EnumWindowsProc]{
    param($hwnd, $lParam)
    if (-not [Native]::IsWindowVisible($hwnd)) { return $true }
    $len = [Native]::GetWindowTextLength($hwnd)
    if ($len -le 0) { return $true }
    $sb = New-Object System.Text.StringBuilder ($len + 1)
    [void][Native]::GetWindowText($hwnd, $sb, $sb.Capacity)
    $title = $sb.ToString()
    if ($title -like '*Codex*') {
      $script:result = $hwnd
      return $false
    }
    return $true
  }
  [void][Native]::EnumWindows($cb, [IntPtr]::Zero)
  return $script:result
}

function Find-CodexAutomationWindow {
  param($Handle)
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

function Find-ElementByNeedles {
  param($Elements, [string[]]$Needles)
  for ($i = 0; $i -lt $Elements.Count; $i++) {
    $el = $Elements.Item($i)
    try {
      $name = Normalize-Name $el.Current.Name
      if (-not $name) { continue }
      foreach ($needle in $Needles) {
        if ($needle -and $name -like "*$needle*") { return $el }
      }
    } catch {}
  }
  return $null
}

function Find-ElementByRegexes {
  param($Elements, [string[]]$Regexes, [string[]]$RejectRegexes)
  for ($i = 0; $i -lt $Elements.Count; $i++) {
    $el = $Elements.Item($i)
    try {
      $name = Normalize-Name $el.Current.Name
      if (-not $name) { continue }
      $reject = $false
      foreach ($rx in $RejectRegexes) {
        if ($rx -and ($name -match $rx)) { $reject = $true; break }
      }
      if ($reject) { continue }
      foreach ($rx in $Regexes) {
        if ($rx -and ($name -match $rx)) { return $el }
      }
    } catch {}
  }
  return $null
}

function Get-DebugCandidateSummary {
  param($Elements)
  $names = New-Object 'System.Collections.Generic.List[string]'
  for ($i = 0; $i -lt $Elements.Count; $i++) {
    $el = $Elements.Item($i)
    try {
      $name = Normalize-Name $el.Current.Name
      if (-not $name) { continue }
      if ($name -match '^1[\.\)]|^2[\.\)]|^3[\.\)]|Yes|Skip|Cancel|remember|ask again') {
        if (-not $names.Contains($name)) {
          [void]$names.Add($name)
        }
      }
    } catch {}
  }
  if ($names.Count -le 0) { return '' }
  return ($names | Select-Object -First 12) -join ' || '
}

function Invoke-Element {
  param($Element)
  if (-not $Element) { return $false }
  try {
    $invoke = $Element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    if ($invoke) {
      $invoke.Invoke()
      return $true
    }
  } catch {}
  try {
    $selection = $Element.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
    if ($selection) {
      $selection.Select()
      return $true
    }
  } catch {}
  try {
    $legacy = $Element.GetCurrentPattern([System.Windows.Automation.LegacyIAccessiblePattern]::Pattern)
    if ($legacy) {
      $legacy.DoDefaultAction()
      return $true
    }
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

$handle = Get-CodexWindowHandle
if ($handle -eq [IntPtr]::Zero) { Write-Output 'NO_CODEX_WINDOW'; exit 3 }
[void][Native]::ShowWindow($handle, 9)
[void][Native]::SetForegroundWindow($handle)
[void][Native]::BringWindowToTop($handle)
Start-Sleep -Milliseconds 200
$win = Find-CodexAutomationWindow $handle
if (-not $win) { $win = [System.Windows.Automation.AutomationElement]::RootElement }
$rootElement = [System.Windows.Automation.AutomationElement]::RootElement

for ($attempt = 0; $attempt -lt 5; $attempt++) {
  $all = Get-AllElements $win
  $allFallback = $null

  if ($decision -eq 'cancel') {
    $targetNeedles = @('Skip', 'Cancel')
    $targetRegexes = @()
    $rejectRegexes = @()
  } elseif ($decision -eq 'decline-message') {
    $targetNeedles = @()
    $targetRegexes = @('^3[\.\)]\s*')
    $rejectRegexes = @('^1[\.\)]\s*', '^2[\.\)]\s*')
  } elseif ($decision -eq 'accept-remember') {
    $targetNeedles = @("don''t ask again", 'remember')
    $targetRegexes = @('^2[\.\)]\s*')
    $rejectRegexes = @('^1[\.\)]\s*', '^3[\.\)]\s*')
  } else {
    $targetNeedles = @('1. Yes', 'Yes')
    $targetRegexes = @('^1[\.\)]\s*')
    $rejectRegexes = @('^2[\.\)]\s*', '^3[\.\)]\s*')
  }

  if ($targetRegexes.Count -gt 0) {
    $option = Find-ElementByRegexes $all $targetRegexes $rejectRegexes
  } else {
    $option = $null
  }
  if (-not $option) {
    $option = Find-ElementByNeedles $all $targetNeedles
  }
  if (-not $option -and $win -ne $rootElement) {
    if (-not $allFallback) { $allFallback = Get-AllElements $rootElement }
    if ($targetRegexes.Count -gt 0) {
      $option = Find-ElementByRegexes $allFallback $targetRegexes $rejectRegexes
    }
    if (-not $option) {
      $option = Find-ElementByNeedles $allFallback $targetNeedles
    }
  }

  if ($option -and (Invoke-Element $option)) {
    Start-Sleep -Milliseconds 180
    Write-Output ("ACTION=" + $decision)
    exit 0
  }

  Start-Sleep -Milliseconds 180
}

if ($allFallback) {
  $debugSummary = Get-DebugCandidateSummary $allFallback
} else {
  $debugSummary = Get-DebugCandidateSummary $all
}
if ($debugSummary) {
  Write-Output ("APPROVAL_CONTROL_NOT_FOUND " + $debugSummary)
} else {
  Write-Output 'APPROVAL_CONTROL_NOT_FOUND'
}
exit 6