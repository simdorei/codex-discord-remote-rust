"""Codex thread state resolution helpers for the Discord bridge."""

from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from codex_thread_models import ThreadInfo

LogFunc = Callable[[str], None]
ResolveSelectedTargetFunc = Callable[[], tuple[str | None, str]]
ResolveTargetRefFunc = Callable[[str | None], tuple[str | None, str]]
GetSelectedInteractiveStateFunc = Callable[[], tuple[str, str | None, str]]


class ThreadStateBridge(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...
    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...
    def get_thread_busy_state(self, thread: ThreadInfo, *, allow_resume: bool = False) -> str | None: ...


def resolve_selected_target(
    *,
    bridge_module: ThreadStateBridge,
    log_func: LogFunc | None = None,
) -> tuple[str | None, str]:
    try:
        thread = bridge_module.choose_thread(None, None)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        if log_func is not None:
            log_func(f"resolve_selected_target_failed error={exc}")
        return None, ""
    return thread.id, bridge_module.get_thread_workspace_ref(thread)


def get_selected_interactive_state(
    *,
    bridge_module: ThreadStateBridge,
    resolve_selected_target_func: ResolveSelectedTargetFunc,
    state_none: str,
    state_input: str,
    state_approval: str,
    log_func: LogFunc | None = None,
) -> tuple[str, str | None, str]:
    target_thread_id, target_ref = resolve_selected_target_func()
    if not target_thread_id:
        return state_none, None, target_ref
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
        busy_state = bridge_module.get_thread_busy_state(thread, allow_resume=True)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        if log_func is not None:
            log_func(f"selected_interactive_state_check_failed target={target_thread_id} error={exc}")
        return state_none, target_thread_id, target_ref
    if busy_state not in {state_input, state_approval}:
        return state_none, target_thread_id, target_ref
    return busy_state, target_thread_id, target_ref


def resolve_target_ref(
    target_thread_id: str | None,
    *,
    bridge_module: ThreadStateBridge,
    resolve_selected_target_func: ResolveSelectedTargetFunc,
    log_func: LogFunc | None = None,
) -> tuple[str | None, str]:
    if not target_thread_id:
        return resolve_selected_target_func()
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
        return thread.id, bridge_module.get_thread_workspace_ref(thread)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        if log_func is not None:
            log_func(f"resolve_target_ref_failed target={target_thread_id} error={exc}")
        return target_thread_id, target_thread_id[:8]


def get_interactive_state_for_thread(
    target_thread_id: str | None,
    *,
    bridge_module: ThreadStateBridge,
    resolve_target_ref_func: ResolveTargetRefFunc,
    get_selected_interactive_state_func: GetSelectedInteractiveStateFunc,
    state_none: str,
    state_input: str,
    state_approval: str,
    log_func: LogFunc | None = None,
) -> tuple[str, str | None, str]:
    if not target_thread_id:
        return get_selected_interactive_state_func()
    target_thread_id, target_ref = resolve_target_ref_func(target_thread_id)
    if not target_thread_id:
        return state_none, None, target_ref
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
        busy_state = bridge_module.get_thread_busy_state(thread, allow_resume=True)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        if log_func is not None:
            log_func(f"interactive_state_check_failed target={target_thread_id} error={exc}")
        return state_none, target_thread_id, target_ref
    if busy_state not in {state_input, state_approval}:
        return state_none, target_thread_id, target_ref
    return busy_state, target_thread_id, target_ref


def get_busy_state_for_thread(
    target_thread_id: str | None,
    *,
    bridge_module: ThreadStateBridge,
    resolve_target_ref_func: ResolveTargetRefFunc,
    log_func: LogFunc,
) -> tuple[str, str | None, str]:
    target_thread_id, target_ref = resolve_target_ref_func(target_thread_id)
    if not target_thread_id:
        return "idle", None, target_ref
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
        busy_state = bridge_module.get_thread_busy_state(thread, allow_resume=True)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        log_func(f"busy_state_check_failed target={target_thread_id} error={exc}")
        return "idle", target_thread_id, target_ref
    return busy_state or "idle", target_thread_id, target_ref
