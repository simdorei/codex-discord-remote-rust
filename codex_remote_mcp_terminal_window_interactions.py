from __future__ import annotations

import base64
import os
import secrets
import threading
from datetime import UTC, datetime
from collections.abc import Callable
from typing import final

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_interaction_types import (
    TerminalWindowInteractionBackend,
    TerminalWindowObservation,
    terminal_window_identity_digest,
)
from codex_remote_mcp_terminal_window_types import (
    OwnedTerminalWindow,
    TerminalWindowBackend,
)
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowAction,
    TerminalWindowActionOutput,
    TerminalWindowActionReceipt,
    TerminalWindowActivateRequest,
    TerminalWindowCaptureOutput,
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalWindowEntry

ActionResult = tuple[bool, tuple[str, ...]]
ActionPerformer = Callable[[OwnedTerminalWindow], ActionResult]


@final
class TerminalWindowInteractionController:
    def __init__(
        self,
        lock: threading.RLock,
        windows: dict[str, OwnedTerminalWindow],
        lifecycle: TerminalWindowBackend,
        interaction: TerminalWindowInteractionBackend,
    ) -> None:
        self._lock = lock
        self._windows = windows
        self._lifecycle = lifecycle
        self._interaction = interaction
        self._observations: dict[str, TerminalWindowObservation] = {}

    def capture(
        self,
        request: TerminalWindowCaptureRequest,
    ) -> TerminalWindowCaptureOutput:
        with self._lock:
            owned, _ = self._owned(request.terminal_window_id)
            captured = self._interaction.capture(owned)
            _, current = self._owned(request.terminal_window_id)
            observation_id = f"twobs_{secrets.token_hex(8)}"
            digest = terminal_window_identity_digest(owned)
            if owned.window_process_id is None:
                raise TerminalExecutionError("terminal window identity is unavailable")
            observation = TerminalWindowObservation(
                observation_id=observation_id,
                terminal_window_id=request.terminal_window_id,
                identity_digest=digest,
                window_process_id=owned.window_process_id,
                rect=captured.rect,
            )
            if not self._interaction.matches_observation(owned, observation):
                raise TerminalExecutionError("terminal window changed during capture")
            self._observations[request.terminal_window_id] = observation
            return TerminalWindowCaptureOutput(
                window=current,
                observation_id=observation_id,
                identity_digest=digest,
                rect=captured.rect,
                data_base64=base64.b64encode(captured.png).decode("ascii"),
                captured_at=datetime.now(UTC),
            )

    def activate(
        self,
        request: TerminalWindowActivateRequest,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            owned, _ = self._owned(request.terminal_window_id)
            activated = self._interaction.activate(owned)
            _, current = self._owned(request.terminal_window_id)
            self.drop(request.terminal_window_id)
            return self._action_output(
                owned,
                current,
                action="activate",
                observation_id=None,
                activated=activated,
            )

    def type_text(
        self,
        request: TerminalWindowTypeRequest,
    ) -> TerminalWindowActionOutput:
        return self._observed_action(
            request.terminal_window_id,
            request.observation_id,
            "type",
            lambda owned: (self._interaction.type_text(owned, request.text), ()),
            unicode_chars=len(request.text),
        )

    def press_keys(
        self,
        request: TerminalWindowKeysRequest,
    ) -> TerminalWindowActionOutput:
        return self._observed_action(
            request.terminal_window_id,
            request.observation_id,
            "keys",
            lambda owned: self._interaction.press_keys(owned, request.keys),
        )

    def interrupt(
        self,
        request: TerminalWindowInterruptRequest,
    ) -> TerminalWindowActionOutput:
        def perform(owned: OwnedTerminalWindow) -> tuple[bool, tuple[str, ...]]:
            self._interaction.interrupt(owned)
            return False, ("CTRL", "C")

        return self._observed_action(
            request.terminal_window_id,
            request.observation_id,
            "interrupt",
            perform,
        )

    def drop(self, terminal_window_id: str) -> None:
        _ = self._observations.pop(terminal_window_id, None)

    def clear(self) -> None:
        self._observations.clear()

    def _observed_action(
        self,
        terminal_window_id: str,
        observation_id: str,
        action: TerminalWindowAction,
        perform: ActionPerformer,
        *,
        unicode_chars: int = 0,
    ) -> TerminalWindowActionOutput:
        with self._lock:
            owned, _ = self._owned(terminal_window_id)
            observation = self._observations.get(terminal_window_id)
            if (
                observation is None
                or observation.observation_id != observation_id
                or not self._interaction.matches_observation(owned, observation)
            ):
                raise TerminalExecutionError(
                    "terminal window changed after capture; take a fresh capture"
                )
            try:
                activated, keys = perform(owned)
                _, current = self._owned(terminal_window_id)
            finally:
                self.drop(terminal_window_id)
            return self._action_output(
                owned,
                current,
                action=action,
                observation_id=observation.observation_id,
                activated=activated,
                unicode_chars=unicode_chars,
                keys=keys,
            )

    def _owned(
        self,
        terminal_window_id: str,
    ) -> tuple[OwnedTerminalWindow, TerminalWindowEntry]:
        self._interaction.require_supported()
        owned = self._windows.get(terminal_window_id)
        if owned is None:
            raise TerminalExecutionError(
                "terminal window does not belong to this session"
            )
        entry = self._lifecycle.inspect(owned)
        if entry is None:
            self._lifecycle.close(owned)
            del self._windows[terminal_window_id]
            self.drop(terminal_window_id)
            raise TerminalExecutionError("terminal window is no longer available")
        return owned, entry

    def _action_output(
        self,
        owned: OwnedTerminalWindow,
        entry: TerminalWindowEntry,
        *,
        action: TerminalWindowAction,
        observation_id: str | None,
        activated: bool,
        unicode_chars: int = 0,
        keys: tuple[str, ...] = (),
    ) -> TerminalWindowActionOutput:
        return TerminalWindowActionOutput(
            window=entry,
            receipt=TerminalWindowActionReceipt(
                receipt_id=f"twrcpt_{secrets.token_hex(8)}",
                terminal_window_id=entry.terminal_window_id,
                observation_id=observation_id,
                identity_digest=terminal_window_identity_digest(owned),
                action=action,
                unicode_chars=unicode_chars,
                keys=keys,
                activated=activated,
                completed_at=datetime.now(UTC),
            ),
        )


def default_terminal_window_interaction_backend() -> TerminalWindowInteractionBackend:
    if os.name != "nt":
        from codex_remote_mcp_terminal_window_interaction_unsupported import (
            UnsupportedTerminalWindowInteractionBackend,
        )

        return UnsupportedTerminalWindowInteractionBackend()
    from codex_remote_mcp_terminal_window_interaction_windows import (
        WindowsTerminalWindowInteractionBackend,
    )

    return WindowsTerminalWindowInteractionBackend()


__all__ = [
    "TerminalWindowInteractionController",
    "default_terminal_window_interaction_backend",
]
