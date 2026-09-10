from __future__ import annotations

from dataclasses import dataclass, replace

import pytest

import codex_remote_mcp_windows_input as windows_input
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry


@dataclass(frozen=True, slots=True)
class FakePermit:
    identity: ComputerWindowIdentity

    def require_active(self) -> None:
        return None


def _identity() -> ComputerWindowIdentity:
    return ComputerWindowIdentity(
        window_id=42,
        process_id=123,
        process_path=r"c:\windows\system32\notepad.exe",
        title_digest="a" * 64,
        left=10,
        top=20,
        width=800,
        height=600,
        surface_window_id=84,
        surface_digest="b" * 64,
        surface_left=20,
        surface_top=50,
        surface_width=760,
        surface_height=520,
    )


def _permit() -> FakePermit:
    return FakePermit(_identity())


def _resolved() -> ResolvedWindow:
    return ResolvedWindow(
        entry=ComputerWindowEntry(
            window_id=42,
            title="Notepad",
            process_name="notepad.exe",
            left=10,
            top=20,
            width=800,
            height=600,
            active=False,
        ),
        identity=_identity(),
    )


def _mutated() -> ResolvedWindow:
    resolved = _resolved()
    return ResolvedWindow(
        entry=resolved.entry,
        identity=replace(
            resolved.identity,
            title_digest="d" * 64,
            surface_digest="e" * 64,
        ),
    )


def _resolve_identity(_: ComputerWindowIdentity) -> ResolvedWindow:
    return _resolved()


def _accept_message(
    window_id: int,
    message: int,
    wparam: int = 0,
    lparam: int = 0,
) -> int:
    _ = window_id, message, wparam, lparam
    return 1


def test_click_targets_the_notepad_edit_control_without_global_mouse_input(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sent: list[tuple[int, int, int, int]] = []
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        lambda _: _resolved(),
    )
    monkeypatch.setattr(
        windows_input,
        "_send_message",
        lambda hwnd, message, wparam=0, lparam=0: sent.append(
            (hwnd, message, wparam, lparam)
        ),
    )
    monkeypatch.setattr(
        windows_input.USER32,
        "mouse_event",
        lambda *_: pytest.fail("global mouse input must never be used"),
        raising=False,
    )

    windows_input.click_window(_permit(), 20, 40, "left", 1)

    assert [item[0] for item in sent] == [84, 84]


def test_ctrl_a_uses_a_window_bound_edit_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    sent: list[tuple[int, int, int, int]] = []
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        lambda _: _resolved(),
    )
    monkeypatch.setattr(
        windows_input,
        "_send_message",
        lambda hwnd, message, wparam=0, lparam=0: sent.append(
            (hwnd, message, wparam, lparam)
        ),
    )
    monkeypatch.setattr(
        windows_input.USER32,
        "SendInput",
        lambda *_: pytest.fail("global keyboard input must never be used"),
        raising=False,
    )

    windows_input.press_window_keys(_permit(), ("CTRL", "A"))

    assert sent == [(84, windows_input.EM_SETSEL, 0, -1)]


def test_text_input_accepts_post_action_content_digest_changes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    post_checks: list[ComputerWindowIdentity] = []
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        lambda _: _resolved(),
    )

    def verify_mutation(identity: ComputerWindowIdentity) -> ResolvedWindow:
        post_checks.append(identity)
        return _mutated()

    monkeypatch.setattr(
        windows_input,
        "require_same_active_window_after_mutation",
        verify_mutation,
    )
    monkeypatch.setattr(windows_input, "_send_message", lambda *_: 1)

    windows_input.type_window_text(_permit(), "QA Unicode: 한글 ✓")

    assert post_checks == [_identity()]


@pytest.mark.parametrize("keys", (("CTRL", "Z"), ("BACKSPACE",)))
def test_mutating_keys_accept_post_action_content_digest_changes(
    monkeypatch: pytest.MonkeyPatch,
    keys: tuple[str, ...],
) -> None:
    post_checks: list[ComputerWindowIdentity] = []
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        _resolve_identity,
    )

    def verify_mutation(identity: ComputerWindowIdentity) -> ResolvedWindow:
        post_checks.append(identity)
        return _mutated()

    monkeypatch.setattr(
        windows_input,
        "require_same_active_window_after_mutation",
        verify_mutation,
    )
    monkeypatch.setattr(windows_input, "_send_message", _accept_message)

    windows_input.press_window_keys(_permit(), keys)

    assert post_checks == [_identity()]


def test_copy_keeps_the_exact_post_action_content_check(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        _resolve_identity,
    )
    monkeypatch.setattr(
        windows_input,
        "require_same_active_window_after_mutation",
        lambda _: pytest.fail("copy must not allow content identity changes"),
    )
    monkeypatch.setattr(windows_input, "_send_message", _accept_message)

    windows_input.press_window_keys(_permit(), ("CTRL", "C"))


def test_alt_tab_is_rejected_before_any_window_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        lambda _: _resolved(),
    )
    monkeypatch.setattr(
        windows_input,
        "_send_message",
        lambda *_: pytest.fail("protected chord must not be sent"),
    )

    with pytest.raises(ComputerControlError, match="protected keyboard shortcut"):
        windows_input.press_window_keys(_permit(), ("ALT", "TAB"))


def test_chrome_scroll_is_rejected_before_any_window_message(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    process_path = r"c:\program files\google\chrome\application\chrome.exe"
    identity = replace(
        _identity(),
        process_path=process_path,
        surface_window_id=None,
        surface_width=0,
        surface_height=0,
    )
    chrome = ResolvedWindow(
        entry=_resolved().entry.model_copy(
            update={"title": "Chrome", "process_name": "chrome.exe"}
        ),
        identity=identity,
    )
    monkeypatch.setattr(
        windows_input,
        "require_matching_active_window",
        lambda _: chrome,
    )
    monkeypatch.setattr(
        windows_input,
        "_send_message",
        lambda *_: pytest.fail("Chrome scroll must not be sent"),
    )

    with pytest.raises(ComputerControlError, match="direct user control"):
        windows_input.scroll_window(FakePermit(identity), 20, 40, 0, 120)
