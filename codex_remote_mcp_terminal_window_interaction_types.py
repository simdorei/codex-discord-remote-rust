from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Protocol

from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowObservationId,
    TerminalWindowRect,
)


@dataclass(frozen=True, slots=True)
class TerminalWindowCapture:
    rect: TerminalWindowRect
    png: bytes


@dataclass(frozen=True, slots=True)
class TerminalWindowObservation:
    observation_id: TerminalWindowObservationId
    terminal_window_id: str
    identity_digest: str
    window_process_id: int
    rect: TerminalWindowRect


class TerminalWindowInteractionBackend(Protocol):
    def require_supported(self) -> None: ...
    def capture(self, window: OwnedTerminalWindow) -> TerminalWindowCapture: ...
    def activate(self, window: OwnedTerminalWindow) -> bool: ...
    def type_text(self, window: OwnedTerminalWindow, text: str) -> bool: ...
    def press_keys(
        self,
        window: OwnedTerminalWindow,
        keys: tuple[str, ...],
    ) -> tuple[bool, tuple[str, ...]]: ...
    def interrupt(self, window: OwnedTerminalWindow) -> None: ...
    def matches_observation(
        self,
        window: OwnedTerminalWindow,
        observation: TerminalWindowObservation,
    ) -> bool: ...


def terminal_window_identity_digest(window: OwnedTerminalWindow) -> str:
    material = (
        f"{window.entry.terminal_window_id}\0{window.entry.window_id}\0"
        + str(window.window_process_id)
    )
    return hashlib.sha256(material.encode()).hexdigest()


__all__ = [
    "TerminalWindowCapture",
    "TerminalWindowInteractionBackend",
    "TerminalWindowObservation",
    "terminal_window_identity_digest",
]
