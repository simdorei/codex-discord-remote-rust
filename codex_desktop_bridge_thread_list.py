from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from codex_bridge_state import JsonObject
from codex_thread_models import ThreadContextUsage, ThreadInfo


class SidecarClient(Protocol):
    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject: ...

    def close(self) -> None: ...


BuildWorkspaceRefMap = Callable[[list[ThreadInfo]], dict[str, str]]
CollapseListText = Callable[[str, int], str]
FormatTimestamp = Callable[[int], str]
FormatTokenK = Callable[[int], str]
FormatThreadModelDisplay = Callable[[ThreadInfo, str, str], str]
GetLivePendingApprovalDisplayLines = Callable[[ThreadInfo, float], tuple[str | None, list[str]]]
GetPendingInteractiveSummary = Callable[[Path], str]
GetSelectedThreadId = Callable[[], str | None]
GetThreadBusyState = Callable[[ThreadInfo, SidecarClient | None, bool], str]
GetThreadContextUsage = Callable[[ThreadInfo], ThreadContextUsage | None]
GetThreadText = Callable[[ThreadInfo], str]
GetThreadUiName = Callable[[str, ThreadInfo], str | None]
IsThreadBusy = Callable[[Path], bool]
MakeConsoleSafeText = Callable[[str], str]
NewSidecar = Callable[[], SidecarClient]
PrintLine = Callable[[str], None]
ShouldRecommendArchive = Callable[[ThreadInfo, ThreadContextUsage | None], bool]
SummarizeInteractiveLines = Callable[[str | None, list[str]], str]


@dataclass(frozen=True, slots=True)
class ThreadListDeps:
    get_selected_thread_id: GetSelectedThreadId
    build_workspace_ref_map: BuildWorkspaceRefMap
    get_thread_ui_name: GetThreadUiName
    collapse_list_text: CollapseListText
    get_thread_workspace_name: GetThreadText
    is_thread_busy: IsThreadBusy
    new_sidecar: NewSidecar
    get_thread_busy_state: GetThreadBusyState
    get_thread_context_usage: GetThreadContextUsage
    format_token_k: FormatTokenK
    should_recommend_archive: ShouldRecommendArchive
    format_thread_model_display: FormatThreadModelDisplay
    get_thread_collaboration_mode: GetThreadText
    get_thread_service_tier: GetThreadText
    format_timestamp: FormatTimestamp
    make_console_safe_text: MakeConsoleSafeText
    get_live_pending_approval_display_lines: GetLivePendingApprovalDisplayLines
    summarize_interactive_lines: SummarizeInteractiveLines
    get_pending_interactive_summary: GetPendingInteractiveSummary
    print_line: PrintLine


def print_thread_list(threads: list[ThreadInfo], deps: ThreadListDeps) -> None:
    selected_thread_id = deps.get_selected_thread_id()
    workspace_refs = deps.build_workspace_ref_map(threads)
    sidecar: SidecarClient | None = None
    try:
        for index, thread in enumerate(threads, start=1):
            if index > 1:
                deps.print_line("")
            session_path = Path(thread.rollout_path)
            if sidecar is None and session_path.exists() and deps.is_thread_busy(session_path):
                try:
                    sidecar = deps.new_sidecar()
                except RuntimeError:
                    sidecar = None
            _print_thread_row(index, thread, selected_thread_id, workspace_refs, sidecar, deps)
    finally:
        if sidecar is not None:
            sidecar.close()


def _print_thread_row(
    index: int,
    thread: ThreadInfo,
    selected_thread_id: str | None,
    workspace_refs: dict[str, str],
    sidecar: SidecarClient | None,
    deps: ThreadListDeps,
) -> None:
    marker = "*" if thread.id == selected_thread_id else " "
    ui_name = deps.get_thread_ui_name(thread.id, thread)
    summary = deps.collapse_list_text(ui_name or thread.title, 70)
    workspace = workspace_refs.get(thread.id, deps.get_thread_workspace_name(thread))
    session_path = Path(thread.rollout_path)
    state = _thread_state(thread, session_path, sidecar, deps)
    context_usage = deps.get_thread_context_usage(thread)
    ctx_display = _context_display(context_usage, deps.format_token_k)
    used_display = deps.format_token_k(thread.tokens_used)
    rec_display = "archive" if deps.should_recommend_archive(thread, context_usage) else "-"
    model_display = deps.format_thread_model_display(
        thread,
        deps.get_thread_collaboration_mode(thread),
        deps.get_thread_service_tier(thread),
    )
    line = (
        f"{marker}{index:>2} | {workspace:<12} | {state:<16} | "
        f"ctx {ctx_display:>15} | used {used_display:>7} | rec {rec_display:<7} | "
        f"model {model_display:<36} | "
        f"{thread.id} | {deps.format_timestamp(thread.updated_at)} | {summary}"
    )
    deps.print_line(deps.make_console_safe_text(line))
    _print_waiting_summary(thread, session_path, state, deps)


def _thread_state(
    thread: ThreadInfo,
    session_path: Path,
    sidecar: SidecarClient | None,
    deps: ThreadListDeps,
) -> str:
    if not session_path.exists() or not deps.is_thread_busy(session_path):
        return "idle"
    return deps.get_thread_busy_state(thread, sidecar, True) if sidecar else "busy"


def _context_display(
    context_usage: ThreadContextUsage | None,
    format_token_k: FormatTokenK,
) -> str:
    if context_usage is None:
        return "-/-"
    return f"{format_token_k(context_usage.last_input_tokens)}/{format_token_k(context_usage.peak_input_tokens)}"


def _print_waiting_summary(
    thread: ThreadInfo,
    session_path: Path,
    state: str,
    deps: ThreadListDeps,
) -> None:
    if not state.startswith("waiting-"):
        return
    interactive_summary = ""
    if state == "waiting-approval":
        live_state, live_lines = deps.get_live_pending_approval_display_lines(thread, 0.75)
        interactive_summary = deps.summarize_interactive_lines(live_state, live_lines)
    if not interactive_summary:
        interactive_summary = deps.get_pending_interactive_summary(session_path)
    if interactive_summary:
        deps.print_line(deps.make_console_safe_text(f"     | request      | {interactive_summary}"))


def print_archived_thread_list(threads: list[ThreadInfo], deps: ThreadListDeps) -> None:
    selected_thread_id = deps.get_selected_thread_id()
    workspace_refs = deps.build_workspace_ref_map(threads)
    for index, thread in enumerate(threads, start=1):
        if index > 1:
            deps.print_line("")
        marker = "*" if thread.id == selected_thread_id else " "
        summary = deps.collapse_list_text(thread.title, 70)
        workspace = workspace_refs.get(thread.id, deps.get_thread_workspace_name(thread))
        archived_at = deps.format_timestamp(thread.archived_at or thread.updated_at)
        line = f"{marker}{index:>2} | {workspace:<12} | {thread.id} | {archived_at} | {summary}"
        deps.print_line(deps.make_console_safe_text(line))
