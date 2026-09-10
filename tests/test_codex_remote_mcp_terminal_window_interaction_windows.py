from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

import pytest

import codex_remote_mcp_terminal_window_interaction_windows as interaction_windows
from codex_remote_mcp_terminal_windows import TerminalWindowManager
from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_input_windows import normalize_terminal_keys
from codex_remote_mcp_terminal_window_interaction_windows import (
    WindowsTerminalWindowInteractionBackend,
)
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowRect,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowEntry,
    TerminalWindowOpenRequest,
)

pytestmark = pytest.mark.skipif(os.name != "nt", reason="Windows-only real input QA")


class _FakeOwnedProcess:
    _process_id: int

    def __init__(self, process_id: int) -> None:
        self._process_id = process_id

    @property
    def pid(self) -> int:
        return self._process_id

    def poll(self) -> int | None:
        return None

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return 0

    def terminate(self) -> None:
        return None

    def kill(self) -> None:
        return None


def test_terminal_key_policy_normalizes_chords_and_blocks_system_keys() -> None:
    assert normalize_terminal_keys(("l", "ctrl")) == ("CTRL", "L")
    with pytest.raises(TerminalExecutionError, match="system-wide"):
        _ = normalize_terminal_keys(("WIN", "R"))


def test_interrupt_targets_retained_bootstrap_process(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    bootstrap_process_id = 101
    window_process_id = 202
    recorded_arguments: list[str] = []

    def fake_run(
        arguments: list[str],
        **_: object,
    ) -> subprocess.CompletedProcess[bytes]:
        recorded_arguments.extend(arguments)
        return subprocess.CompletedProcess(arguments, 0, b"", b"")

    def fake_rect(_: OwnedTerminalWindow) -> TerminalWindowRect:
        return TerminalWindowRect(left=0, top=0, width=800, height=600)

    monkeypatch.setattr(
        interaction_windows,
        "require_terminal_window_rect",
        fake_rect,
    )
    monkeypatch.setattr(subprocess, "run", fake_run)
    window = OwnedTerminalWindow(
        entry=TerminalWindowEntry(
            terminal_window_id="termwin_0123456789abcdef",
            window_id=303,
            process_id=bootstrap_process_id,
            shell="powershell",
            cwd="C:/qa",
            title="MCP QA",
        ),
        process=_FakeOwnedProcess(bootstrap_process_id),
        window_process_id=window_process_id,
    )

    WindowsTerminalWindowInteractionBackend().interrupt(window)

    assert recorded_arguments[-1] == str(bootstrap_process_id)
    assert recorded_arguments[-1] != str(window_process_id)


def test_real_cmd_capture_type_and_enter_creates_marker(tmp_path: Path) -> None:
    manager = TerminalWindowManager(tmp_path)
    try:
        opened = manager.open(TerminalWindowOpenRequest(shell="cmd"))
        window_id = opened.window.terminal_window_id
        marker = tmp_path / "cmd-terminal-input.txt"
        command = f'> "{marker}" echo terminal-input-ok'

        first = manager.capture(
            TerminalWindowCaptureRequest(terminal_window_id=window_id)
        )
        typed = manager.type_text(
            TerminalWindowTypeRequest(
                terminal_window_id=window_id,
                observation_id=first.observation_id,
                text=command,
            )
        )
        second = manager.capture(
            TerminalWindowCaptureRequest(terminal_window_id=window_id)
        )
        entered = manager.press_keys(
            TerminalWindowKeysRequest(
                terminal_window_id=window_id,
                observation_id=second.observation_id,
                keys=("ENTER",),
            )
        )

        _wait_for_text(marker, "terminal-input-ok")
        assert first.data_base64.startswith("iVBOR")
        assert typed.receipt.unicode_chars == len(command)
        assert typed.receipt.activated is True
        assert entered.receipt.keys == ("ENTER",)
        assert entered.receipt.activated is True
    finally:
        manager.close_all()


def test_real_powershell_interrupt_returns_control_to_owned_shell(
    tmp_path: Path,
) -> None:
    manager = TerminalWindowManager(tmp_path)
    try:
        opened = manager.open(TerminalWindowOpenRequest(shell="powershell"))
        window_id = opened.window.terminal_window_id
        started = tmp_path / "powershell-sleep-started.txt"
        completed = tmp_path / "powershell-sleep-completed.txt"
        escaped_started = str(started).replace("'", "''")
        escaped_completed = str(completed).replace("'", "''")
        _submit(
            manager,
            window_id,
            "; ".join(
                (
                    f"Set-Content -LiteralPath '{escaped_started}' -Value 'started'",
                    "Start-Sleep -Seconds 120",
                    f"Set-Content -LiteralPath '{escaped_completed}' -Value 'completed'",
                )
            ),
        )
        _wait_for_text(started, "started")

        observed = manager.capture(
            TerminalWindowCaptureRequest(terminal_window_id=window_id)
        )
        interrupted = manager.interrupt(
            TerminalWindowInterruptRequest(
                terminal_window_id=window_id,
                observation_id=observed.observation_id,
            )
        )
        marker = tmp_path / "powershell-after-interrupt.txt"
        escaped = str(marker).replace("'", "''")
        _submit(
            manager,
            window_id,
            f"Set-Content -LiteralPath '{escaped}' -Value 'interrupt-ok'",
        )

        _wait_for_text(marker, "interrupt-ok", timeout=12)
        assert not completed.exists()
        assert any(
            window.terminal_window_id == window_id for window in manager.list().windows
        )
        assert interrupted.receipt.action == "interrupt"
        assert interrupted.receipt.keys == ("CTRL", "C")
    finally:
        manager.close_all()


def _submit(manager: TerminalWindowManager, window_id: str, command: str) -> None:
    observed = manager.capture(
        TerminalWindowCaptureRequest(terminal_window_id=window_id)
    )
    _ = manager.type_text(
        TerminalWindowTypeRequest(
            terminal_window_id=window_id,
            observation_id=observed.observation_id,
            text=command,
        )
    )
    observed = manager.capture(
        TerminalWindowCaptureRequest(terminal_window_id=window_id)
    )
    _ = manager.press_keys(
        TerminalWindowKeysRequest(
            terminal_window_id=window_id,
            observation_id=observed.observation_id,
            keys=("ENTER",),
        )
    )


def _wait_for_text(path: Path, expected: str, *, timeout: float = 8) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if path.exists() and expected in path.read_text(encoding="utf-8-sig"):
            return
        time.sleep(0.05)
    pytest.fail(f"terminal input did not create the expected marker: {path.name}")
