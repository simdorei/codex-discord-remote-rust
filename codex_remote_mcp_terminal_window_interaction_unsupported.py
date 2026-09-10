from __future__ import annotations

from typing import final, override

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_interaction_types import (
    TerminalWindowCapture,
    TerminalWindowInteractionBackend,
    TerminalWindowObservation,
)
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow


@final
class UnsupportedTerminalWindowInteractionBackend(TerminalWindowInteractionBackend):
    @override
    def require_supported(self) -> None:
        raise TerminalExecutionError(
            "visible terminal window interaction is supported only on Windows"
        )

    @override
    def capture(self, window: OwnedTerminalWindow) -> TerminalWindowCapture:
        _ = window
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def activate(self, window: OwnedTerminalWindow) -> bool:
        _ = window
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def type_text(self, window: OwnedTerminalWindow, text: str) -> bool:
        _ = window, text
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def press_keys(
        self,
        window: OwnedTerminalWindow,
        keys: tuple[str, ...],
    ) -> tuple[bool, tuple[str, ...]]:
        _ = window, keys
        self.require_supported()
        raise AssertionError("unreachable")

    @override
    def interrupt(self, window: OwnedTerminalWindow) -> None:
        _ = window
        self.require_supported()

    @override
    def matches_observation(
        self,
        window: OwnedTerminalWindow,
        observation: TerminalWindowObservation,
    ) -> bool:
        _ = window, observation
        self.require_supported()
        raise AssertionError("unreachable")


__all__ = ["UnsupportedTerminalWindowInteractionBackend"]
