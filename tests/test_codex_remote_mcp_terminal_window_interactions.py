from __future__ import annotations

from pathlib import Path
from typing import final

import pytest

from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_interaction_types import (
    TerminalWindowCapture,
    TerminalWindowObservation,
)
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowActivateRequest,
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowRect,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowEntry,
    TerminalWindowOpenRequest,
    TerminalWindowShell,
)


@final
class FakeProcess:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.exit_code: int | None = None

    def poll(self) -> int | None:
        return self.exit_code

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return self.exit_code or 0

    def terminate(self) -> None:
        self.exit_code = 0

    def kill(self) -> None:
        self.exit_code = 1


@final
class FakeLifecycle:
    def __init__(self) -> None:
        self.next_pid = 200

    def require_supported(self) -> None:
        return None

    def open(
        self,
        terminal_window_id: str,
        shell: TerminalWindowShell,
        cwd: Path,
        title: str,
    ) -> OwnedTerminalWindow:
        self.next_pid += 1
        entry = TerminalWindowEntry(
            terminal_window_id=terminal_window_id,
            window_id=self.next_pid + 1_000,
            process_id=self.next_pid,
            shell=shell,
            cwd=str(cwd),
            title=title,
        )
        return OwnedTerminalWindow(
            entry=entry,
            process=FakeProcess(self.next_pid),
            window_process_id=self.next_pid + 2_000,
        )

    def inspect(self, window: OwnedTerminalWindow) -> TerminalWindowEntry | None:
        return window.entry if window.process.poll() is None else None

    def close(self, window: OwnedTerminalWindow) -> None:
        window.process.terminate()


@final
class FakeInteraction:
    def __init__(self) -> None:
        self.typed: list[str] = []
        self.interrupts = 0
        self.matches = True
        self.fail_type = False

    def require_supported(self) -> None:
        return None

    def capture(self, window: OwnedTerminalWindow) -> TerminalWindowCapture:
        _ = window
        return TerminalWindowCapture(
            rect=TerminalWindowRect(left=10, top=20, width=640, height=480),
            png=b"\x89PNG\r\n\x1a\nsynthetic",
        )

    def activate(self, window: OwnedTerminalWindow) -> bool:
        _ = window
        return True

    def type_text(self, window: OwnedTerminalWindow, text: str) -> bool:
        _ = window
        if self.fail_type:
            raise TerminalExecutionError("synthetic input failure")
        self.typed.append(text)
        return False

    def press_keys(
        self,
        window: OwnedTerminalWindow,
        keys: tuple[str, ...],
    ) -> tuple[bool, tuple[str, ...]]:
        _ = window
        return False, tuple(key.upper() for key in keys)

    def interrupt(self, window: OwnedTerminalWindow) -> None:
        _ = window
        self.interrupts += 1

    def matches_observation(
        self,
        window: OwnedTerminalWindow,
        observation: TerminalWindowObservation,
    ) -> bool:
        return self.matches and (
            window.window_process_id == observation.window_process_id
        )


def test_capture_then_type_returns_redacted_shape_receipt(tmp_path: Path) -> None:
    manager, interaction, window_id = _manager(tmp_path)
    captured = manager.capture(TerminalWindowCaptureRequest(terminal_window_id=window_id))
    secret_text = "not-stored-in-receipt"

    output = manager.type_text(
        TerminalWindowTypeRequest(
            terminal_window_id=window_id,
            observation_id=captured.observation_id,
            text=secret_text,
        )
    )

    assert interaction.typed == [secret_text]
    assert output.receipt.action == "type"
    assert output.receipt.unicode_chars == len(secret_text)
    assert secret_text not in output.model_dump_json()
    assert output.receipt.observation_id == captured.observation_id
    with pytest.raises(TerminalExecutionError, match="fresh capture"):
        _ = manager.type_text(
            TerminalWindowTypeRequest(
                terminal_window_id=window_id,
                observation_id=captured.observation_id,
                text="replay",
            )
        )


def test_keys_interrupt_activate_and_stale_observation(tmp_path: Path) -> None:
    manager, interaction, window_id = _manager(tmp_path)
    first = manager.capture(TerminalWindowCaptureRequest(terminal_window_id=window_id))
    keys = manager.press_keys(
        TerminalWindowKeysRequest(
            terminal_window_id=window_id,
            observation_id=first.observation_id,
            keys=("ctrl", "l"),
        )
    )
    second = manager.capture(TerminalWindowCaptureRequest(terminal_window_id=window_id))
    interrupted = manager.interrupt(
        TerminalWindowInterruptRequest(
            terminal_window_id=window_id,
            observation_id=second.observation_id,
        )
    )
    third = manager.capture(TerminalWindowCaptureRequest(terminal_window_id=window_id))
    interaction.matches = False

    assert keys.receipt.keys == ("CTRL", "L")
    assert interrupted.receipt.keys == ("CTRL", "C")
    assert interaction.interrupts == 1
    with pytest.raises(TerminalExecutionError, match="fresh capture"):
        _ = manager.interrupt(
            TerminalWindowInterruptRequest(
                terminal_window_id=window_id,
                observation_id=third.observation_id,
            )
        )
    interaction.matches = True
    activated = manager.activate(
        TerminalWindowActivateRequest(terminal_window_id=window_id)
    )
    assert activated.receipt.action == "activate"
    assert activated.receipt.observation_id is None


def test_failed_input_consumes_observation(tmp_path: Path) -> None:
    manager, interaction, window_id = _manager(tmp_path)
    captured = manager.capture(TerminalWindowCaptureRequest(terminal_window_id=window_id))
    request = TerminalWindowTypeRequest(
        terminal_window_id=window_id,
        observation_id=captured.observation_id,
        text="failure",
    )
    interaction.fail_type = True

    with pytest.raises(TerminalExecutionError, match="synthetic input failure"):
        _ = manager.type_text(request)
    interaction.fail_type = False
    with pytest.raises(TerminalExecutionError, match="fresh capture"):
        _ = manager.type_text(request)


def _manager(
    root: Path,
) -> tuple[TerminalWindowManager, FakeInteraction, str]:
    lifecycle = FakeLifecycle()
    interaction = FakeInteraction()
    manager = TerminalWindowManager(
        root,
        backend=lifecycle,
        interaction_backend=interaction,
    )
    opened = manager.open(TerminalWindowOpenRequest(shell="cmd"))
    return manager, interaction, opened.window.terminal_window_id
