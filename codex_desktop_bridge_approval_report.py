from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Protocol

from codex_bridge_state import JsonObject
from codex_thread_models import ThreadInfo


class ReplyToPendingApproval(Protocol):
    def __call__(self, thread: ThreadInfo, answer_text: str, timeout_sec: float = 6.0) -> JsonObject: ...


ChooseThread = Callable[[str | None, str | None], ThreadInfo]
GetThreadWorkspaceRef = Callable[[ThreadInfo], str]
PrintLine = Callable[[str], None]
ResolveThreadRef = Callable[[str], ThreadInfo]


@dataclass(frozen=True, slots=True)
class ApprovalReportDeps:
    get_thread_workspace_ref: GetThreadWorkspaceRef
    print_line: PrintLine


@dataclass(frozen=True, slots=True)
class ApprovalReplyCommandDeps:
    choose_thread: ChooseThread
    resolve_thread_ref: ResolveThreadRef
    reply_to_pending_approval: ReplyToPendingApproval
    get_thread_workspace_ref: GetThreadWorkspaceRef
    print_line: PrintLine


def run_approval_reply_command(
    *,
    thread_ref: str,
    thread_id: str | None,
    cwd: str | None,
    answer: str,
    timeout: float,
    deps: ApprovalReplyCommandDeps,
) -> None:
    thread = deps.resolve_thread_ref(thread_ref) if thread_ref else deps.choose_thread(thread_id, cwd)
    result = deps.reply_to_pending_approval(thread, answer, timeout_sec=timeout)
    print_approval_reply_result(
        thread,
        result,
        ApprovalReportDeps(
            get_thread_workspace_ref=deps.get_thread_workspace_ref,
            print_line=deps.print_line,
        ),
    )


def print_approval_reply_result(
    thread: ThreadInfo,
    result: JsonObject,
    deps: ApprovalReportDeps,
) -> None:
    deps.print_line(f"thread_id: {thread.id}")
    deps.print_line(f"thread_ref: {deps.get_thread_workspace_ref(thread)}")
    deps.print_line(f"decision_action: {result.get('decision_action') or '-'}")
    deps.print_line(f"request_kind: {result.get('request_kind') or '-'}")
    deps.print_line(f"request_id: {result.get('request_id') or '-'}")
    verification_busy_state = str(result.get("verification_busy_state") or "").strip()
    if verification_busy_state:
        deps.print_line(f"verification_busy_state: {verification_busy_state}")
    attempts = result.get("attempts")
    if isinstance(attempts, list) and attempts:
        deps.print_line("attempts:")
        for entry in attempts:
            deps.print_line(f"- {entry}")
