from __future__ import annotations

import pytest

import codex_remote_mcp_windows_device_platform as device_platform
from codex_remote_mcp_computer_contracts import ComputerCapture, ComputerWindowIdentity
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
        window_id=81,
        title="Existing ERP administration",
        process_name="erp-admin.exe",
        left=10,
        top=20,
        width=900,
        height=700,
        active=True,
    )
    return ResolvedWindow(
        entry=entry,
        identity=ComputerWindowIdentity(
            window_id=entry.window_id,
            process_id=1234,
            process_path=r"c:\program files\erp\erp-admin.exe",
            title_digest="a" * 64,
            left=entry.left,
            top=entry.top,
            width=entry.width,
            height=entry.height,
        ),
    )


def test_device_platform_lists_preexisting_windows(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    monkeypatch.setattr(
        device_platform,
        "list_device_windows",
        lambda: (resolved.entry,),
    )

    platform = device_platform.WindowsDeviceComputerPlatform()

    assert platform.list_windows() == (resolved.entry,)


def test_device_platform_captures_a_preexisting_window(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    capture = ComputerCapture(
        window=resolved.entry,
        identity=resolved.identity,
        png=b"device-window-png",
    )
    monkeypatch.setattr(device_platform, "resolve_device_window", lambda _: resolved)
    monkeypatch.setattr(device_platform, "capture_device_window", lambda _: capture)

    platform = device_platform.WindowsDeviceComputerPlatform()

    assert platform.screenshot(resolved.entry.window_id) == capture


def test_device_platform_close_targets_the_observed_existing_window(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    resolved = _resolved()
    permit = _Permit(resolved.identity)
    posted: list[tuple[int, int]] = []
    monkeypatch.setattr(
        device_platform,
        "require_matching_active_device_window",
        lambda _: resolved,
    )
    monkeypatch.setattr(
        device_platform.USER32,
        "PostMessageW",
        lambda window_id, message, _wparam, _lparam: posted.append(
            (window_id, message)
        )
        or True,
    )

    platform = device_platform.WindowsDeviceComputerPlatform()
    platform.close(permit)

    assert posted == [(resolved.entry.window_id, device_platform.WM_CLOSE)]
    assert permit.checks == 1
