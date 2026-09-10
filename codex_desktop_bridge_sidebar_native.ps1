$code = @'
using System;
using System.Runtime.InteropServices;
public static class Native {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int maxCount);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, int dwData, UIntPtr dwExtraInfo);
  [DllImport("user32.dll")] public static extern void keybd_event(byte bVk, byte bScan, uint dwFlags, UIntPtr dwExtraInfo);
}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type $code

function Add-SidebarActivationError {
  param([string]$Stage, $ErrorRecord)
  if (-not $script:SidebarActivationErrors) {
    $script:SidebarActivationErrors = New-Object System.Collections.Generic.List[string]
  }
  $message = $ErrorRecord.Exception.Message
  $errorType = $ErrorRecord.Exception.GetType().Name
  [void]$script:SidebarActivationErrors.Add("${Stage}:${errorType}:${message}")
}

function Get-SidebarActivationErrors {
  if (-not $script:SidebarActivationErrors) { return '' }
  return ($script:SidebarActivationErrors -join ' | ')
}

function Test-CodexWindowHandle {
  param([IntPtr]$Handle)
  if ($Handle -eq [IntPtr]::Zero -or -not [Native]::IsWindow($Handle) -or -not [Native]::IsWindowVisible($Handle)) {
    return $false
  }
  $sb = New-Object System.Text.StringBuilder 512
  [void][Native]::GetWindowText($Handle, $sb, $sb.Capacity)
  $title = (($sb.ToString() -replace "`r|`n", ' ') -replace '\s+', ' ').Trim()
  if ($title -eq 'Codex' -or $title -like 'Codex - *') { return $true }
  if ($title -ne 'ChatGPT') { return $false }

  [uint32]$ownerProcessId = 0
  [void][Native]::GetWindowThreadProcessId($Handle, [ref]$ownerProcessId)
  try {
    $ownerPath = (Get-Process -Id $ownerProcessId -ErrorAction Stop).Path
  } catch {
    return $false
  }
  $windowsAppsRoot = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'WindowsApps'
  $expectedPath = '^{0}\\OpenAI\.Codex_[^\\]+\\app\\ChatGPT\.exe$' -f [Regex]::Escape($windowsAppsRoot.TrimEnd('\'))
  return [Regex]::IsMatch($ownerPath, $expectedPath, [Text.RegularExpressions.RegexOptions]::IgnoreCase)
}

function Get-CodexWindowHandle {
  $windowHandle = 0L
  if ([long]::TryParse($env:CODEX_WINDOW_HANDLE, [ref]$windowHandle) -and $windowHandle -ne 0) {
    $suppliedHandle = [IntPtr]$windowHandle
    if (Test-CodexWindowHandle $suppliedHandle) { return $suppliedHandle }
    return [IntPtr]::Zero
  }
  $script:result = [IntPtr]::Zero
  $cb = [Native+EnumWindowsProc]{
param($hWnd, $lParam)
if (Test-CodexWindowHandle $hWnd) { $script:result = $hWnd; return $false }
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

function Normalize-Name {
  param([string]$Name)
  return (($Name -replace "`r|`n", ' ') -replace '\s+', ' ').Trim()
}

function Refresh-AllElements {
  param($Window)
  return Get-AllElements $Window
}

function Invoke-Or-Click {
  param($Element)
  $pattern = $null
  if ($Element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) {
try { $pattern.Select(); return $true } catch { Add-SidebarActivationError 'selection_select' $_ }
  }
  $pattern = $null
  if ($Element.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
try { $pattern.Invoke(); return $true } catch { Add-SidebarActivationError 'invoke_pattern' $_ }
  }
  try {
$point = $Element.GetClickablePoint()
[void][Native]::SetCursorPos([int]$point.X, [int]$point.Y)
Start-Sleep -Milliseconds 80
[Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
[Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
return $true
  } catch { Add-SidebarActivationError 'clickable_point' $_ }
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
  } catch { Add-SidebarActivationError 'bounding_rect_click' $_ }
  return $false
}

function Send-Key {
  param([byte]$VirtualKey)
  [Native]::keybd_event($VirtualKey, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 45
  [Native]::keybd_event($VirtualKey, 0, 0x0002, [UIntPtr]::Zero)
}

function Send-Hotkey {
  param([byte[]]$Keys)
  foreach ($key in $Keys) {
[Native]::keybd_event($key, 0, 0, [UIntPtr]::Zero)
  }
  Start-Sleep -Milliseconds 50
  for ($i = $Keys.Length - 1; $i -ge 0; $i--) {
[Native]::keybd_event($Keys[$i], 0, 0x0002, [UIntPtr]::Zero)
  }
}
