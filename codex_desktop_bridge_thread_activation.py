from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_thread_models import ThreadInfo

GetThreadUiNameCandidates = Callable[[ThreadInfo], list[str]]
VerifyHeader = Callable[[str], str | None]
VerifyThread = Callable[[str], str | None]
ActivateThreadDeepLink = Callable[[str], None]
Clock = Callable[[], float]
Sleep = Callable[[float], None]


class ActivateSidebar(Protocol):
    def __call__(self, thread_name: str, project_name: str | None = None) -> str: ...


class WaitForThreadActivation(Protocol):
    def __call__(self, thread: ThreadInfo, thread_name: str, timeout_sec: float = 5.0) -> str | None: ...


class ThreadActivationError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class ThreadActivationDeps:
    get_thread_ui_name_candidates: GetThreadUiNameCandidates
    verify_active_thread_by_header: VerifyHeader
    verify_active_thread: VerifyThread
    activate_thread_deep_link: ActivateThreadDeepLink
    activate_thread_by_sidebar_v2: ActivateSidebar
    wait_for_thread_activation: WaitForThreadActivation
    now: Clock
    sleep: Sleep


def wait_for_thread_activation(
    thread: ThreadInfo,
    thread_name: str,
    *,
    timeout_sec: float = 5.0,
    deps: ThreadActivationDeps,
) -> str | None:
    deadline = deps.now() + max(0.25, timeout_sec)
    attempt = 0
    while True:
        header_verified = deps.verify_active_thread_by_header(thread_name)
        if header_verified:
            return header_verified

        verified_by = deps.verify_active_thread(thread.id)
        if verified_by:
            return verified_by

        now = deps.now()
        if now >= deadline:
            return None

        sleep_for = min(0.75, 0.2 + (attempt * 0.15), max(0.05, deadline - now))
        deps.sleep(sleep_for)
        attempt += 1


def activate_thread_in_ui(thread: ThreadInfo, deps: ThreadActivationDeps) -> str:
    ui_name_candidates = deps.get_thread_ui_name_candidates(thread)
    for thread_name in ui_name_candidates:
        header_verified = deps.verify_active_thread_by_header(thread_name)
        if header_verified:
            return f"already-open [{header_verified}]"

    verified_by = deps.verify_active_thread(thread.id)
    if verified_by:
        return f"already-open [{verified_by}]"

    last_error = ""
    verification_name = ui_name_candidates[0] if ui_name_candidates else ""
    try:
        activation_method = activate_thread_via_deep_link(thread, deps)
    except OSError as exc:
        last_error = f"Exact Codex thread deep link failed: {exc}"
    else:
        verified_by = deps.wait_for_thread_activation(thread, verification_name, timeout_sec=8.0)
        if verified_by:
            return f"{activation_method} [{verified_by}]"
        last_error = "Opened the exact Codex thread deep link, but the Codex UI did not confirm the active thread."

    for thread_name in ui_name_candidates:
        try:
            matched_label = deps.activate_thread_by_sidebar_v2(
                thread_name,
                Path(thread.cwd).name if thread.cwd else None,
            )
        except RuntimeError as exc:
            last_error = str(exc)
            continue

        verified_by = deps.wait_for_thread_activation(thread, thread_name, timeout_sec=5.0)
        if verified_by:
            return f"sidebar:{matched_label} [{verified_by}]"

        last_error = "Clicked the sidebar thread item, but the Codex UI did not confirm the active thread."

    if ui_name_candidates:
        raise ThreadActivationError(last_error or "Unable to activate the target thread in the Codex UI sidebar.")

    raise ThreadActivationError(
        "Unable to activate the target thread in the Codex UI sidebar because no usable UI label was found."
    )


def activate_thread_via_deep_link(thread: ThreadInfo, deps: ThreadActivationDeps) -> str:
    deps.activate_thread_deep_link(thread.id)
    return "deeplink:codex"


def verify_thread_in_ui(thread: ThreadInfo, deps: ThreadActivationDeps) -> str | None:
    for thread_name in deps.get_thread_ui_name_candidates(thread):
        header_verified = deps.verify_active_thread_by_header(thread_name)
        if header_verified:
            return header_verified
    return deps.verify_active_thread(thread.id)
