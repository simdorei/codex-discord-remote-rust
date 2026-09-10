from __future__ import annotations

from dataclasses import dataclass
from collections.abc import Callable
from enum import StrEnum
from typing import Protocol

from simdorei_mcp_common.operation_outputs import ComputerWindowEntry


class ComputerAccessMode(StrEnum):
    PROJECT = "project"
    DEVICE = "device"


@dataclass(frozen=True, slots=True)
class ComputerWindowIdentity:
    window_id: int
    process_id: int
    process_path: str
    title_digest: str
    left: int
    top: int
    width: int
    height: int
    surface_window_id: int | None = None
    surface_digest: str = ""
    surface_left: int = 0
    surface_top: int = 0
    surface_width: int = 0
    surface_height: int = 0


class ComputerActionPermit(Protocol):
    @property
    def identity(self) -> ComputerWindowIdentity: ...
    def require_active(self) -> None: ...


@dataclass(frozen=True, slots=True)
class ComputerCapture:
    window: ComputerWindowEntry
    identity: ComputerWindowIdentity
    png: bytes


class ComputerPlatform(Protocol):
    def stop(self, *, deadline_monotonic: float | None = None) -> None: ...
    def list_windows(self) -> tuple[ComputerWindowEntry, ...]: ...
    def screenshot(self, window_id: int) -> ComputerCapture: ...
    def activate(self, window_id: int) -> ComputerWindowEntry: ...
    def launch(
        self,
        app: str,
        *,
        ensure_active: Callable[[], None] | None = None,
    ) -> None: ...
    def close(self, permit: ComputerActionPermit) -> None: ...
    def click(
        self, permit: ComputerActionPermit, x: int, y: int, button: str, click_count: int
    ) -> None: ...
    def drag(
        self,
        permit: ComputerActionPermit,
        start_x: int,
        start_y: int,
        end_x: int,
        end_y: int,
    ) -> None: ...
    def scroll(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        delta_x: int,
        delta_y: int,
    ) -> None: ...
    def type_text(self, permit: ComputerActionPermit, text: str) -> None: ...
    def press_keys(self, permit: ComputerActionPermit, keys: tuple[str, ...]) -> None: ...
    def set_clipboard(self, permit: ComputerActionPermit, text: str) -> None: ...
