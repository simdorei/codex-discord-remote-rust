from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from codex_thread_models import ThreadInfo

GetActiveTurnId = Callable[[str], str | None]
InterruptTurn = Callable[[str, str], object]
GetThreadLabel = Callable[[ThreadInfo], str]
Sleep = Callable[[float], None]
TimeNow = Callable[[], float]
PrintLine = Callable[[str], None]

STOP_CONFIRM_TIMEOUT_SEC = 3.0
STOP_CONFIRM_POLL_SEC = 0.2


@dataclass(frozen=True, slots=True)
class StopCommandDeps:
    get_active_turn_id: GetActiveTurnId
    interrupt_turn: InterruptTurn
    get_thread_label: GetThreadLabel
    time_now: TimeNow
    sleep: Sleep
    print_line: PrintLine


def run_stop_command(thread: ThreadInfo, *, deps: StopCommandDeps) -> None:
    label = deps.get_thread_label(thread)
    deps.print_line(f"target_thread: {label}")
    turn_id = deps.get_active_turn_id(thread.id)
    requested = bool(turn_id)
    if turn_id:
        _ = deps.interrupt_turn(thread.id, turn_id)
    deps.print_line(f"reply_stop_requested: {str(requested).lower()}")
    if not requested:
        deps.print_line("reply_stop_confirmed: false")
        return
    deps.print_line(f"reply_stop_confirmed: {str(_wait_for_thread_idle(thread.id, deps)).lower()}")


def _wait_for_thread_idle(thread_id: str, deps: StopCommandDeps) -> bool:
    deadline = deps.time_now() + STOP_CONFIRM_TIMEOUT_SEC
    while deps.time_now() < deadline:
        if deps.get_active_turn_id(thread_id) is None:
            return True
        deps.sleep(STOP_CONFIRM_POLL_SEC)
    return deps.get_active_turn_id(thread_id) is None
