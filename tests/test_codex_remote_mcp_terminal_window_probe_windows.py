from __future__ import annotations

import os
from collections.abc import Callable, Iterator

import pytest

import codex_remote_mcp_terminal_window_probe_windows as probe
from codex_remote_mcp_terminal_engine import TerminalExecutionError
from codex_remote_mcp_terminal_window_types import OwnedTerminalWindow
from simdorei_mcp_common.terminal_window_protocol import TerminalWindowEntry

pytestmark = pytest.mark.skipif(os.name != "nt", reason="Windows-only probe QA")
_PROCESS_ID = 84


class _Process:
    pid: int = _PROCESS_ID

    def poll(self) -> int | None:
        return None

    def wait(self, timeout: float | None = None) -> int:
        _ = timeout
        return 0

    def terminate(self) -> None:
        return None

    def kill(self) -> None:
        return None


class _LiveUser32:
    @staticmethod
    def IsWindow(_window_id: int) -> bool:
        return True


class _GoneUser32:
    @staticmethod
    def IsWindow(_window_id: int) -> bool:
        return False


def test_initial_pid_disappearance_is_stale(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(probe, "USER32", _LiveUser32())
    monkeypatch.setattr(probe, "try_window_process_id", _missing_pid)

    assert probe.inspect_owned_terminal_window(_owned()) is None


def test_snapshot_close_and_post_snapshot_reuse_are_stale(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(probe, "USER32", _LiveUser32())
    process_ids = iter((_PROCESS_ID, None))
    monkeypatch.setattr(probe, "try_window_process_id", _next_value(process_ids))
    monkeypatch.setattr(probe, "terminal_window_entry", _failed_snapshot)

    assert probe.inspect_owned_terminal_window(_owned()) is None

    process_ids = iter((_PROCESS_ID, _PROCESS_ID + 1))
    monkeypatch.setattr(probe, "try_window_process_id", _next_value(process_ids))
    monkeypatch.setattr(probe, "terminal_window_entry", _successful_snapshot)

    assert probe.inspect_owned_terminal_window(_owned()) is None


def test_live_same_process_snapshot_failure_is_not_hidden(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(probe, "USER32", _LiveUser32())
    process_ids = iter((_PROCESS_ID, _PROCESS_ID))
    monkeypatch.setattr(probe, "try_window_process_id", _next_value(process_ids))
    monkeypatch.setattr(probe, "terminal_window_entry", _failed_snapshot)

    with pytest.raises(TerminalExecutionError, match="inspect"):
        _ = probe.inspect_owned_terminal_window(_owned())


def test_try_pid_only_converts_a_disappeared_window_to_stale(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        probe,
        "require_window_process_id",
        _pid_error,
    )
    monkeypatch.setattr(probe, "USER32", _GoneUser32())
    assert probe.try_window_process_id(42) is None

    monkeypatch.setattr(probe, "USER32", _LiveUser32())
    with pytest.raises(TerminalExecutionError, match="identify"):
        _ = probe.try_window_process_id(42)


def _owned() -> OwnedTerminalWindow:
    return OwnedTerminalWindow(
        entry=_entry(),
        process=_Process(),
        window_process_id=_PROCESS_ID,
    )


def _entry() -> TerminalWindowEntry:
    return TerminalWindowEntry(
        terminal_window_id="termwin_0123456789abcdef",
        window_id=42,
        process_id=_PROCESS_ID,
        shell="cmd",
        cwd="C:/qa",
        title="Codex Pro Terminal",
    )


def _next_value(values: Iterator[int | None]) -> Callable[[int], int | None]:
    return lambda _window_id: next(values)


def _missing_pid(_window_id: int) -> None:
    return None


def _successful_snapshot(*_args: object) -> TerminalWindowEntry:
    return _entry()


def _failed_snapshot(*_args: object) -> TerminalWindowEntry:
    raise TerminalExecutionError("Windows could not inspect the terminal window")


def _raise_pid_error() -> int:
    raise TerminalExecutionError("Windows could not identify the terminal window")


def _pid_error(_window_id: int) -> int:
    return _raise_pid_error()
