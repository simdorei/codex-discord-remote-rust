from __future__ import annotations

import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_thread_models import WindowInfo

WindowCallback = Callable[[int], bool]
EnumWindows = Callable[[WindowCallback], None]
GetWindowTextLength = Callable[[int], int]
ReadWindowText = Callable[[int, int], str]
GetWindowProcessPath = Callable[[int], str]
GetWindowRect = Callable[[int], tuple[int, int, int, int] | None]
NativeWindowCall = Callable[[int], None]
Sleep = Callable[[float], None]
SendKeyEvent = Callable[[int, bool], None]


class WindowFocusError(RuntimeError):
    pass


class RunProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        capture_output: bool,
        text: bool,
        creationflags: int,
        timeout: float,
        check: bool,
    ) -> subprocess.CompletedProcess[str]: ...


@dataclass(frozen=True, slots=True)
class WindowTextDeps:
    get_window_text_length: GetWindowTextLength
    read_window_text: ReadWindowText


@dataclass(frozen=True, slots=True)
class WindowFocusDeps:
    enum_windows: EnumWindows
    is_window_visible: Callable[[int], bool]
    get_window_text: Callable[[int], str]
    get_window_process_path: GetWindowProcessPath
    get_window_rect: GetWindowRect
    get_foreground_window: Callable[[], int]
    show_window: NativeWindowCall
    set_foreground_window: NativeWindowCall
    bring_window_to_top: NativeWindowCall
    run_process: RunProcess
    send_key_event: SendKeyEvent
    sleep: Sleep
    restore_command: int
    tab_key: int
    create_no_window: int = 0


COMPOSER_FOCUS_SCRIPT = r"""
$code = @'
using System;
using System.Runtime.InteropServices;
public static class Native {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int maxCount);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);
}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type $code
$script:result = [IntPtr]::Zero
$cb = [Native+EnumWindowsProc]{
  param($hWnd, $lParam)
  if (-not [Native]::IsWindowVisible($hWnd)) { return $true }
  $sb = New-Object System.Text.StringBuilder 512
  [void][Native]::GetWindowText($hWnd, $sb, $sb.Capacity)
  $title = $sb.ToString().Trim()
  $isCodexTitle = $title -eq 'Codex' -or $title -like 'Codex - *'
  $isCodexAppx = $false
  if ($title -eq 'ChatGPT') {
    [uint32]$ownerProcessId = 0
    [void][Native]::GetWindowThreadProcessId($hWnd, [ref]$ownerProcessId)
    $ownerPath = (Get-Process -Id $ownerProcessId -ErrorAction SilentlyContinue).Path
    $isCodexAppx = $ownerPath -like '*\WindowsApps\OpenAI.Codex_*\app\ChatGPT.exe'
  }
  if ($isCodexTitle -or $isCodexAppx) { $script:result = $hWnd; return $false }
  return $true
}
[void][Native]::EnumWindows($cb, [IntPtr]::Zero)
if ($script:result -eq [IntPtr]::Zero) { Write-Output 'NO_CODEX_WINDOW'; exit 2 }
$cond = New-Object System.Windows.Automation.PropertyCondition(
  [System.Windows.Automation.AutomationElement]::NativeWindowHandleProperty,
  [int]$script:result
)
$win = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst(
  [System.Windows.Automation.TreeScope]::Descendants,
  $cond
)
if (-not $win) { Write-Output 'NO_AUTOMATION_WINDOW'; exit 3 }
$all = $win.FindAll(
  [System.Windows.Automation.TreeScope]::Descendants,
  [System.Windows.Automation.Condition]::TrueCondition
)
foreach ($el in $all) {
  if ($el.Current.ClassName -like 'ProseMirror*' -and $el.Current.IsKeyboardFocusable) {
    try {
      $el.SetFocus()
      Start-Sleep -Milliseconds 120
      $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
      if ($focused -and $focused.Current.ClassName -like 'ProseMirror*') {
        Write-Output 'OK'
        exit 0
      }
    } catch {}
  }
}
Write-Output 'NO_PROSEMIRROR'
exit 4
"""


def get_window_text(hwnd: int, deps: WindowTextDeps) -> str:
    length = deps.get_window_text_length(hwnd)
    if length <= 0:
        return ""
    return deps.read_window_text(hwnd, length + 1)


def is_codex_desktop_window_title(title: str) -> bool:
    normalized = " ".join(title.strip().split())
    return normalized == "Codex" or normalized.startswith("Codex - ")


def is_codex_desktop_appx_window(title: str, process_path: str) -> bool:
    normalized_title = " ".join(title.strip().split())
    normalized_path = process_path.strip().replace("/", "\\").casefold()
    if normalized_path.startswith("\\\\?\\"):
        normalized_path = normalized_path[4:]
    path_parts = tuple(part for part in normalized_path.split("\\") if part)
    return (
        normalized_title == "ChatGPT"
        and len(path_parts) == 6
        and path_parts[0].endswith(":")
        and path_parts[1] == "program files"
        and path_parts[2] == "windowsapps"
        and path_parts[3].startswith("openai.codex_")
        and path_parts[3] != "openai.codex_"
        and path_parts[4:] == ("app", "chatgpt.exe")
    )


def find_codex_window(deps: WindowFocusDeps) -> WindowInfo:
    found: list[WindowInfo] = []

    def visit(hwnd: int) -> bool:
        if not deps.is_window_visible(hwnd):
            return True

        title = deps.get_window_text(hwnd).strip()
        if not is_codex_desktop_window_title(title) and not is_codex_desktop_appx_window(
            title, deps.get_window_process_path(hwnd)
        ):
            return True

        rect = deps.get_window_rect(hwnd)
        if rect is None:
            return True

        left, top, right, bottom = rect
        found.append(WindowInfo(hwnd=hwnd, title=title, left=left, top=top, right=right, bottom=bottom))
        return True

    deps.enum_windows(visit)
    if not found:
        raise WindowFocusError("Visible Codex Desktop window not found.")

    foreground = deps.get_foreground_window()
    for window in found:
        if window.hwnd == foreground:
            return window
    return found[0]


def focus_window(window: WindowInfo, deps: WindowFocusDeps) -> None:
    deps.show_window(window.hwnd)
    deps.set_foreground_window(window.hwnd)
    deps.bring_window_to_top(window.hwnd)
    deps.sleep(0.2)


def focus_codex_composer(deps: WindowFocusDeps) -> bool:
    try:
        result = deps.run_process(
            ["powershell", "-NoProfile", "-Command", COMPOSER_FOCUS_SCRIPT],
            capture_output=True,
            text=True,
            creationflags=deps.create_no_window,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return False

    output = (result.stdout or "").strip()
    return result.returncode == 0 and output.endswith("OK")


def ensure_codex_composer_focus(attempts: int, deps: WindowFocusDeps) -> bool:
    if focus_codex_composer(deps):
        return True

    for _ in range(attempts):
        deps.send_key_event(deps.tab_key, False)
        deps.send_key_event(deps.tab_key, True)
        deps.sleep(0.08)
        if focus_codex_composer(deps):
            return True

    return False
