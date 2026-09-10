from __future__ import annotations

import subprocess
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_bridge_state import JsonObject
from codex_thread_models import WindowInfo

ReadText = Callable[[Path], str]
GetClipboardText = Callable[[], str | None]
SetClipboardText = Callable[[str], None]
FindCodexWindow = Callable[[], WindowInfo]
FocusWindow = Callable[[WindowInfo], None]
Sleep = Callable[[float], None]
EnvironCopy = Callable[[], dict[str, str]]


class SendHotkey(Protocol):
    def __call__(self, *keys: int) -> None: ...


class SendKeyEvent(Protocol):
    def __call__(self, vk: int, *, keyup: bool = False) -> None: ...


class RunProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        cwd: str,
        env: dict[str, str],
        capture_output: bool,
        text: bool,
        encoding: str,
        creationflags: int,
    ) -> subprocess.CompletedProcess[str]: ...


class PermissionApprovalReplyEmptyError(RuntimeError):
    pass


class PermissionApprovalDeclineMessageRequiredError(RuntimeError):
    pass


class UnsupportedPermissionApprovalDecisionError(RuntimeError):
    pass


class PermissionApprovalUiSubmitError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class PermissionUiDeps:
    approval_script_path: Path
    row_select_script_path: Path
    script_dir: Path
    read_text: ReadText
    get_clipboard_text: GetClipboardText
    set_clipboard_text: SetClipboardText
    find_codex_window: FindCodexWindow
    focus_window: FocusWindow
    send_hotkey: SendHotkey
    send_key_event: SendKeyEvent
    sleep: Sleep
    run_process: RunProcess
    environ_copy: EnvironCopy
    vk_control: int
    vk_v: int
    vk_return: int
    create_no_window: int = 0


def classify_permission_approval_ui_reply(answer_text: str) -> tuple[str, str]:
    normalized = str(answer_text or "").strip()
    lowered = normalized.lower()
    if normalized == "1" or lowered in {"approve", "approved", "accept", "yes", "y", "ok", "예", "네", "승인"}:
        return ("accept", "")
    if normalized == "2":
        return ("accept-remember", "")
    if lowered in {"cancel", "skip", "dismiss", "건너뛰기", "취소"}:
        return ("cancel", "")
    if normalized == "3":
        raise PermissionApprovalDeclineMessageRequiredError("Option 3 needs a decline message. Send the reason text itself.")
    if normalized:
        return ("decline-message", normalized)
    raise PermissionApprovalReplyEmptyError("Approval reply is empty. Send 1, 2, cancel, or a decline message.")


def submit_permission_approval_via_ui(answer_text: str, deps: PermissionUiDeps) -> JsonObject:
    decision_action, decline_message = classify_permission_approval_ui_reply(answer_text)
    if decision_action == "decline-message":
        _paste_decline_message(decline_message, deps, pre_paste_delay_sec=0.0)
        return {
            "decision_action": decision_action,
            "request_kind": "permission",
            "ui_result": "ACTION=decline-message",
        }

    return _run_permission_script(
        decision_action,
        decline_message="",
        script_path=deps.approval_script_path,
        allow_decline_message=False,
        deps=deps,
    )


def submit_permission_approval_via_ui_row_select(answer_text: str, deps: PermissionUiDeps) -> JsonObject:
    decision_action, decline_message = classify_permission_approval_ui_reply(answer_text)
    result = _run_permission_script(
        decision_action,
        decline_message=decline_message,
        script_path=deps.row_select_script_path,
        allow_decline_message=True,
        deps=deps,
    )
    if decision_action == "decline-message":
        _paste_decline_message(decline_message, deps, pre_paste_delay_sec=0.2)
    return result


def _script_action_for_decision(decision_action: str, *, allow_decline_message: bool) -> str:
    action_arg = {
        "accept": "accept",
        "accept-remember": "accept-remember",
        "cancel": "cancel",
    }.get(decision_action)
    if action_arg:
        return action_arg
    if allow_decline_message and decision_action == "decline-message":
        return "decline-message"
    raise UnsupportedPermissionApprovalDecisionError(f"Unsupported permission approval decision: {decision_action}")


def _run_permission_script(
    decision_action: str,
    *,
    decline_message: str,
    script_path: Path,
    allow_decline_message: bool,
    deps: PermissionUiDeps,
) -> JsonObject:
    action_arg = _script_action_for_decision(decision_action, allow_decline_message=allow_decline_message)
    deps.focus_window(deps.find_codex_window())
    script = deps.read_text(script_path)
    env = deps.environ_copy()
    env["CODEX_APPROVAL_DECISION"] = action_arg
    env["CODEX_APPROVAL_DECLINE_MESSAGE"] = decline_message

    try:
        result = deps.run_process(
            ["powershell", "-NoProfile", "-Command", script],
            cwd=str(deps.script_dir),
            env=env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            creationflags=deps.create_no_window,
        )
    except (OSError, RuntimeError, subprocess.SubprocessError) as exc:
        raise PermissionApprovalUiSubmitError(f"Permission approval UI submit failed: {exc}") from exc

    stdout = (result.stdout or "").strip()
    stderr = (result.stderr or "").strip()
    if result.returncode != 0:
        details = stdout or stderr or f"exit={result.returncode}"
        raise PermissionApprovalUiSubmitError(f"Permission approval UI submit failed: {details}")
    return {
        "decision_action": decision_action,
        "request_kind": "permission",
        "ui_result": stdout or "ok",
    }


def _paste_decline_message(decline_message: str, deps: PermissionUiDeps, *, pre_paste_delay_sec: float) -> None:
    original_clipboard = deps.get_clipboard_text()
    try:
        deps.focus_window(deps.find_codex_window())
        if pre_paste_delay_sec > 0:
            deps.sleep(pre_paste_delay_sec)
        deps.set_clipboard_text(decline_message)
        deps.sleep(0.1)
        deps.send_hotkey(deps.vk_control, deps.vk_v)
        deps.sleep(0.2)
        deps.send_key_event(deps.vk_return, keyup=False)
        deps.send_key_event(deps.vk_return, keyup=True)
    finally:
        if original_clipboard is not None:
            try:
                deps.set_clipboard_text(original_clipboard)
            except (OSError, RuntimeError):
                original_clipboard = None
