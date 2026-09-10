from __future__ import annotations

import pytest

import codex_remote_mcp_windows_device_input as device_input
from codex_remote_mcp_computer_contracts import ComputerWindowIdentity
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry


class _Permit:
    def __init__(self, identity: ComputerWindowIdentity) -> None:
        self.identity = identity
        self.checks = 0

    def require_active(self) -> None:
        self.checks += 1


def _resolved() -> ResolvedWindow:
    entry = ComputerWindowEntry(
        window_id=91,
        title="ERP",
        process_name="erp.exe",
        left=100,
        top=200,
        width=800,
        height=600,
        active=True,
    )
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=entry.window_id,
            process_id=4321,
            process_path=r"c:\erp\erp.exe",
            title_digest="b" * 64,
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
        ),
    )


def test_device_click_uses_screenshot_relative_screen_coordinates(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    permit = _Permit(resolved.identity)
    cursor: list[tuple[int, int]] = []
    mouse: list[int] = []
    monkeypatch.setattr(
        device_input,
        "require_matching_active_device_window",
        lambda _: resolved,
    )
    monkeypatch.setattr(
        device_input,
        "require_same_active_device_window_after_mutation",
        lambda _: resolved,
    )
    monkeypatch.setattr(
        device_input.USER32,
        "SetCursorPos",
        lambda x, y: cursor.append((x, y)) or True,
    )
    monkeypatch.setattr(
        device_input.USER32,
        "mouse_event",
        lambda flag, *_: mouse.append(flag),
    )

    device_input.click_device_window(permit, 25, 30, "right", 2)

    assert cursor == [(125, 230)]
    assert mouse == [
        device_input.MOUSEEVENTF_RIGHTDOWN,
        device_input.MOUSEEVENTF_RIGHTUP,
        device_input.MOUSEEVENTF_RIGHTDOWN,
        device_input.MOUSEEVENTF_RIGHTUP,
    ]
    assert permit.checks == 2


def test_device_keys_surface_secure_attention_limit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    permit = _Permit(resolved.identity)
    monkeypatch.setattr(
        device_input,
        "require_matching_active_device_window",
        lambda _: resolved,
    )

    with pytest.raises(ComputerControlError, match="secure-screen"):
        device_input.press_device_keys(permit, ("CTRL", "ALT", "DELETE"))
