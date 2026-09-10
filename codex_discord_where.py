from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from codex_thread_models import ThreadInfo


class WhereBridge(Protocol):
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...
    def get_thread_busy_state(self, thread: ThreadInfo, *, allow_resume: bool = False) -> str | None: ...
    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...
    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None: ...
    def format_token_k(self, value: int) -> str: ...


GetMirroredThreadFunc = Callable[[int | None], str | None]
DescribeProjectChannelFunc = Callable[[int | None], str]
FormatContextUsageLineFunc = Callable[[ThreadInfo], str]


def build_where_message(
    channel_id: int | None,
    *,
    bridge_module: WhereBridge,
    get_mirrored_codex_thread_id_func: GetMirroredThreadFunc,
    describe_mirrored_project_channel_func: DescribeProjectChannelFunc,
    format_context_usage_line_func: FormatContextUsageLineFunc,
) -> str:
    target_thread_id = get_mirrored_codex_thread_id_func(channel_id)
    if not target_thread_id:
        project_message = describe_mirrored_project_channel_func(channel_id)
        return project_message or "This Discord channel is not mapped to a Codex thread."
    try:
        thread = bridge_module.choose_thread(target_thread_id, None)
        busy_state = bridge_module.get_thread_busy_state(thread, allow_resume=True)
        return "\n".join(
            [
                "Mapped Codex thread",
                f"thread_ref: {bridge_module.get_thread_workspace_ref(thread)}",
                f"thread_id: {thread.id}",
                f"title: {bridge_module.get_thread_ui_name(thread.id, thread) or thread.title or '-'}",
                f"cwd: {thread.cwd or '-'}",
                f"state: {busy_state or 'idle'}",
                format_context_usage_line_func(thread),
                f"tokens_used_total: {bridge_module.format_token_k(thread.tokens_used)}",
            ]
        )
    except RuntimeError as exc:
        return f"Mapped Codex thread: {target_thread_id}\nERROR: {exc}"
