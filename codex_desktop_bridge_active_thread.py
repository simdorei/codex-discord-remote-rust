from __future__ import annotations

import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_thread_models import WindowInfo

GetClipboardText = Callable[[], str | None]
SetClipboardText = Callable[[str], None]
FindCodexWindow = Callable[[], WindowInfo]
FocusWindow = Callable[[WindowInfo], None]
Sleep = Callable[[float], None]
TimeNs = Callable[[], int]
EnvironCopy = Callable[[], dict[str, str]]


class SendHotkey(Protocol):
    def __call__(self, *keys: int) -> None: ...


class SendKeyEvent(Protocol):
    def __call__(self, vk: int, keyup: bool = False) -> None: ...


class RunProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        capture_output: bool,
        text: bool,
        encoding: str,
        errors: str,
        creationflags: int,
        timeout: float,
        check: bool,
        env: dict[str, str],
    ) -> subprocess.CompletedProcess[str]: ...


@dataclass(frozen=True, slots=True)
class ActiveThreadDeps:
    get_clipboard_text: GetClipboardText
    set_clipboard_text: SetClipboardText
    find_codex_window: FindCodexWindow
    focus_window: FocusWindow
    send_hotkey: SendHotkey
    send_key_event: SendKeyEvent
    sleep: Sleep
    time_ns: TimeNs
    run_process: RunProcess
    environ_copy: EnvironCopy
    vk_control: int
    vk_menu: int
    vk_l: int
    vk_c: int
    vk_escape: int
    create_no_window: int = 0


HEADER_VERIFICATION_SCRIPT = r"""
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$targetThread = $env:CODEX_THREAD_NAME
$targetThread = if ($targetThread) { $targetThread.Trim() } else { '' }
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
}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type $code

$windowHandle = 0L
if (-not [long]::TryParse($env:CODEX_WINDOW_HANDLE, [ref]$windowHandle) -or $windowHandle -eq 0) {
  Write-Output 'NO_CODEX_WINDOW'
  exit 3
}
$script:result = [IntPtr]$windowHandle
[void][Native]::SetForegroundWindow($script:result)
Start-Sleep -Milliseconds 120
$cond = New-Object System.Windows.Automation.PropertyCondition(
  [System.Windows.Automation.AutomationElement]::NativeWindowHandleProperty,
  [int]$script:result
)
$win = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
  [System.Windows.Automation.TreeScope]::Descendants,
  $cond
)
if (-not $win) { Write-Output 'NO_AUTOMATION_WINDOW'; exit 4 }
$windowRect = $win.Current.BoundingRectangle
$all = $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
for ($i=0; $i -lt $all.Count; $i++) {
  $el = $all.Item($i)
  $type = $el.Current.ControlType.ProgrammaticName
  if ($type -notin @('ControlType.Text','ControlType.Button')) { continue }
  $name = ($el.Current.Name -replace "`r|`n", ' ').Trim()
  if (-not $name) { continue }
  $rect = $el.Current.BoundingRectangle
  if ($rect.Top -gt 240) { continue }
  if ($rect.Width -le 1 -or $rect.Height -le 1) { continue }
  if ($rect.Left -lt ($windowRect.Left + ($windowRect.Width * 0.28))) { continue }
  if ($rect.Right -gt ($windowRect.Right - 32)) { continue }
  if ($rect.Top -lt ($windowRect.Top + 16)) { continue }
  if ($name.Contains($targetThread)) {
    Write-Output ('OK:' + $name)
    exit 0
  }
}
Write-Output 'NO_HEADER_MATCH'
exit 5
"""


def verify_active_thread(thread_id: str, deps: ActiveThreadDeps) -> str | None:
    original = deps.get_clipboard_text()
    try:
        deps.focus_window(deps.find_codex_window())
        for attempt in range(2):
            sentinel = f"__CODEX_BRIDGE__{deps.time_ns()}__L__"
            deps.set_clipboard_text(sentinel)
            deps.send_hotkey(deps.vk_control, deps.vk_menu, deps.vk_l)
            deps.sleep(0.25)
            deeplink = deps.get_clipboard_text() or ""
            if deeplink != sentinel and thread_id in deeplink:
                return "copy-deeplink"

            sentinel = f"__CODEX_BRIDGE__{deps.time_ns()}__C__"
            deps.set_clipboard_text(sentinel)
            deps.send_hotkey(deps.vk_control, deps.vk_menu, deps.vk_c)
            deps.sleep(0.25)
            session_id = deps.get_clipboard_text() or ""
            if session_id != sentinel and thread_id.strip() == session_id.strip():
                return "copy-session-id"

            if attempt == 0:
                deps.send_key_event(deps.vk_escape, keyup=False)
                deps.send_key_event(deps.vk_escape, keyup=True)
                deps.sleep(0.15)
        return None
    finally:
        if original is not None:
            try:
                deps.set_clipboard_text(original)
            except RuntimeError as restore_error:
                restore_error.add_note("Clipboard restore failed after active-thread verification.")


def verify_active_thread_by_header(thread_name: str, deps: ActiveThreadDeps) -> str | None:
    if not thread_name.strip():
        return None

    window = deps.find_codex_window()
    deps.focus_window(window)
    env = deps.environ_copy()
    env["CODEX_THREAD_NAME"] = thread_name
    env["CODEX_WINDOW_HANDLE"] = str(window.hwnd)
    try:
        result = deps.run_process(
            ["powershell", "-NoProfile", "-Command", HEADER_VERIFICATION_SCRIPT],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=deps.create_no_window,
            timeout=12,
            check=False,
            env=env,
        )
    except (OSError, subprocess.SubprocessError, TimeoutError):
        return None

    output = (result.stdout or "").strip()
    if result.returncode == 0 and output.startswith("OK:"):
        return "header"
    return None
