from __future__ import annotations

from dataclasses import dataclass, field
from collections.abc import Callable
from typing import final

from codex_remote_mcp_computer import ComputerController
from codex_remote_mcp_computer_contracts import (
    ComputerActionPermit,
    ComputerCapture,
    ComputerWindowIdentity,
)
from codex_remote_mcp_windows_launch import LaunchedApplication
from codex_remote_mcp_windows_windows import ResolvedWindow
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry

PNG_BYTES = b"\x89PNG\r\n\x1a\ncomputer-test"


@dataclass(frozen=True, slots=True)
class FakeComputerPlatform:
    window: ComputerWindowEntry
    clicks: list[tuple[int, int, int, str, int]] = field(default_factory=list)
    clipboards: list[tuple[int, str]] = field(default_factory=list)
    stops: list[bool] = field(default_factory=list)

    def list_windows(self) -> tuple[ComputerWindowEntry, ...]:
        return (self.window,)

    def screenshot(self, window_id: int) -> ComputerCapture:
        assert window_id == self.window.window_id
        return ComputerCapture(
            window=self.window,
            identity=computer_identity(self.window),
            png=PNG_BYTES,
        )

    def activate(self, window_id: int) -> ComputerWindowEntry:
        assert window_id == self.window.window_id
        return self.window

    def launch(
        self,
        app: str,
        *,
        ensure_active: Callable[[], None] | None = None,
    ) -> None:
        _ = app
        if ensure_active is not None:
            ensure_active()

    def close(self, permit: ComputerActionPermit) -> None:
        assert permit.identity.window_id == self.window.window_id

    def click(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        button: str,
        click_count: int,
    ) -> None:
        permit.require_active()
        self.clicks.append((permit.identity.window_id, x, y, button, click_count))

    def drag(
        self,
        permit: ComputerActionPermit,
        start_x: int,
        start_y: int,
        end_x: int,
        end_y: int,
    ) -> None:
        _ = permit, start_x, start_y, end_x, end_y

    def scroll(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        delta_x: int,
        delta_y: int,
    ) -> None:
        _ = permit, x, y, delta_x, delta_y

    def type_text(self, permit: ComputerActionPermit, text: str) -> None:
        _ = permit, text

    def press_keys(self, permit: ComputerActionPermit, keys: tuple[str, ...]) -> None:
        _ = permit, keys

    def set_clipboard(self, permit: ComputerActionPermit, text: str) -> None:
        self.clipboards.append((permit.identity.window_id, text))

    def stop(self, *, deadline_monotonic: float | None = None) -> None:
        _ = deadline_monotonic
        self.stops.append(True)


def computer_window() -> ComputerWindowEntry:
    return ComputerWindowEntry(
        window_id=42,
        title="Notepad",
        process_name="notepad.exe",
        left=10,
        top=20,
        width=800,
        height=600,
        active=True,
    )


def computer_identity(window: ComputerWindowEntry) -> ComputerWindowIdentity:
    return ComputerWindowIdentity(
        window_id=window.window_id,
        process_id=1234,
        process_path=r"c:\windows\system32\notepad.exe",
        title_digest="a" * 64,
        left=window.left,
        top=window.top,
        width=window.width,
        height=window.height,
    )


def make_controller(platform: FakeComputerPlatform) -> ComputerController:
    return ComputerController(
        platform,
        clock=lambda: 100.0,
        token_factory=lambda: "observation-test-token",
    )


@final
class FakeOwnedProcess:  # MUTABLE_OK: deterministic Windows lifecycle fake.
    def __init__(self, process_id: int = 123) -> None:
        self._process_id: int = process_id
        self.exit_code: int | None = None
        self.killed: bool = False

    @property
    def pid(self) -> int:
        return self._process_id

    def poll(self) -> int | None:
        return self.exit_code

    def replace_process_id_for_test(self, process_id: int) -> None:
        self._process_id = process_id

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        self.exit_code = 0
        return self.exit_code

    def terminate(self) -> None:
        self.exit_code = 0

    def kill(self) -> None:
        self.killed = True
        self.exit_code = 1


def launched_application(
    resolved: ResolvedWindow,
    *,
    process: FakeOwnedProcess | None = None,
    temporary_profile: str | None = None,
) -> LaunchedApplication:
    return LaunchedApplication(
        window=resolved,
        process=process or FakeOwnedProcess(resolved.identity.process_id),
        temporary_profile=temporary_profile,
    )
