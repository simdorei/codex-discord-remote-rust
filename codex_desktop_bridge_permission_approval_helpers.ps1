[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$code = @'
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
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type $code

function Normalize-Name {
  param([string]$Name)
  if (-not $Name) { return '' }
  return (($Name -replace "`r|`n", ' ') -replace '\s+', ' ').Trim()
}

function Add-ApprovalAutomationError {
  param([string]$Stage, $ErrorRecord)
  if (-not $script:ApprovalAutomationErrors) {
    $script:ApprovalAutomationErrors = New-Object 'System.Collections.Generic.List[string]'
  }
  $errorType = $ErrorRecord.Exception.GetType().Name
  $message = $ErrorRecord.Exception.Message
  [void]$script:ApprovalAutomationErrors.Add("${Stage}:${errorType}:${message}")
}

function Get-ApprovalAutomationErrors {
  if (-not $script:ApprovalAutomationErrors) { return '' }
  return ($script:ApprovalAutomationErrors -join ' | ')
}

function Join-Codepoints {
  param([int[]]$Codepoints)
  return -join @($Codepoints | ForEach-Object { [char]$_ })
}

function Get-RememberApprovalNeedles {
  return @("don't ask again", 'remember', (Join-Codepoints @(0xB2E4, 0xC2DC, 0x20, 0xBB3B, 0xC9C0, 0x20, 0xC54A, 0xAE30)))
}

function Get-YesApprovalNeedles {
  return @('1. Yes', 'Yes', (Join-Codepoints @(0xC608)))
}

function Get-NoApprovalNeedles {
  return @('No', (Join-Codepoints @(0xC544, 0xB2C8, 0xC694)))
}

function Get-SkipApprovalNeedles {
  return @('Skip', 'Cancel', (Join-Codepoints @(0xAC74, 0xB108, 0xB6F0, 0xAE30)))
}

function Get-SubmitApprovalNeedles {
  return @('Submit', (Join-Codepoints @(0xC81C, 0xCD9C)))
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
  param($Elements, [string[]]$Needles, [string[]]$RejectNeedles)
  for ($i = 0; $i -lt $Elements.Count; $i++) {
    $el = $Elements.Item($i)
    try {
      $name = Normalize-Name $el.Current.Name
      if (-not $name) { continue }
      $hit = $false
      foreach ($needle in $Needles) {
        if ($needle -and $name -like "*$needle*") { $hit = $true; break }
      }
      if (-not $hit) { continue }
      $reject = $false
      foreach ($needle in $RejectNeedles) {
        if ($needle -and $name -like "*$needle*") { $reject = $true; break }
      }
      if ($reject) { continue }
      return $el
    } catch { Add-ApprovalAutomationError 'needle_match' $_ }
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
    } catch { Add-ApprovalAutomationError 'regex_match' $_ }
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
      if ($name -match '^1[\.\)]|^2[\.\)]|^3[\.\)]|Yes|Submit|Skip|Cancel|remember|ask again') {
        if (-not $names.Contains($name)) {
          [void]$names.Add($name)
        }
      }
    } catch { Add-ApprovalAutomationError 'debug_summary' $_ }
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
  } catch { Add-ApprovalAutomationError 'invoke_pattern' $_ }
  try {
    $selection = $Element.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern)
    if ($selection) {
      $selection.Select()
      return $true
    }
  } catch { Add-ApprovalAutomationError 'selection_pattern' $_ }
  try {
    $legacy = $Element.GetCurrentPattern([System.Windows.Automation.LegacyIAccessiblePattern]::Pattern)
    if ($legacy) {
      $legacy.DoDefaultAction()
      return $true
    }
  } catch { Add-ApprovalAutomationError 'legacy_pattern' $_ }
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
  } catch { Add-ApprovalAutomationError 'bounding_rect_click' $_ }
  return $false
}
