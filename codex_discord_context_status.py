from __future__ import annotations

from collections.abc import Callable
from typing import Protocol

from codex_thread_models import ThreadContextUsage, ThreadInfo


class ContextStatusBridge(Protocol):
    def get_thread_context_usage(self, thread: ThreadInfo) -> ThreadContextUsage | None: ...
    def describe_thread_context_usage(self, context_usage: ThreadContextUsage) -> str: ...
    def should_recommend_archive(
        self,
        thread: ThreadInfo,
        context_usage: ThreadContextUsage | None,
    ) -> bool: ...
    def format_token_k(self, value: int) -> str: ...
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo: ...
    def get_thread_workspace_ref(self, thread: ThreadInfo) -> str: ...
    def get_thread_ui_name(self, thread_id: str, thread: ThreadInfo | None = None) -> str | None: ...
    def load_recent_threads(self, limit: int = 20) -> list[ThreadInfo]: ...


ResolveTargetRefFunc = Callable[[str | None], tuple[str | None, str]]
LogFunc = Callable[[str], None]
GetMirroredThreadFunc = Callable[[int | None], str | None]
ResolveSelectedTargetFunc = Callable[[], tuple[str | None, str]]


def format_context_usage_line(thread: ThreadInfo, *, bridge_module: ContextStatusBridge) -> str:
    context_usage = bridge_module.get_thread_context_usage(thread)
    if context_usage is None:
        return "context: -"
    status = bridge_module.describe_thread_context_usage(context_usage)
    peak_ratio = (
        context_usage.peak_input_tokens / context_usage.model_context_window
        if context_usage.model_context_window > 0
        else 0.0
    )
    no_visible_reply_state = context_usage.last_total_tokens == 0 and peak_ratio >= 0.90
    if no_visible_reply_state:
        status = f"no-visible-reply, peak={peak_ratio * 100:.1f}%"
    archive_hint = "yes" if bridge_module.should_recommend_archive(thread, context_usage) else "no"
    compaction_hint = f"compactions={context_usage.inferred_compactions}"
    if context_usage.inferred_compactions:
        compaction_hint += (
            f" last={bridge_module.format_token_k(context_usage.last_compaction_before_input_tokens)}"
            f"->{bridge_module.format_token_k(context_usage.last_compaction_after_input_tokens)}"
        )
    return (
        f"context: {context_usage.usage_ratio * 100:.1f}% ({status}) "
        f"last={bridge_module.format_token_k(context_usage.last_input_tokens)} "
        f"peak={bridge_module.format_token_k(context_usage.peak_input_tokens)} "
        f"window={bridge_module.format_token_k(context_usage.model_context_window)} "
        f"{compaction_hint} "
        f"archive_recommended={archive_hint}"
    )


def build_context_warning(
    target_thread_id: str | None,
    *,
    bridge_module: ContextStatusBridge,
    resolve_target_ref_func: ResolveTargetRefFunc,
    log_func: LogFunc,
) -> str:
    try:
        resolved_thread_id, _target_ref = resolve_target_ref_func(target_thread_id)
        if not resolved_thread_id:
            return ""
        thread = bridge_module.choose_thread(resolved_thread_id, None)
        context_usage = bridge_module.get_thread_context_usage(thread)
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        log_func(f"context_warning_unavailable target={target_thread_id or '-'} error={exc}")
        return ""
    if context_usage is None:
        return ""
    status = bridge_module.describe_thread_context_usage(context_usage)
    archive_recommended = bridge_module.should_recommend_archive(thread, context_usage)
    has_compaction_history = context_usage.inferred_compactions > 0
    peak_ratio = (
        context_usage.peak_input_tokens / context_usage.model_context_window
        if context_usage.model_context_window > 0
        else 0.0
    )
    if context_usage.last_total_tokens == 0 and peak_ratio >= 0.90:
        return (
            f"Context warning: no-visible-reply state, peak={peak_ratio * 100:.1f}%, "
            f"archive_recommended={'yes' if archive_recommended else 'no'}, "
            f"token_used_total={bridge_module.format_token_k(thread.tokens_used)}. "
            "Archive this thread and run `!mirror sync` before resending."
        )
    if status not in {"high", "critical"}:
        return ""
    compaction_note = ""
    if has_compaction_history:
        compaction_note = (
            f" compactions={context_usage.inferred_compactions}"
            f" last={bridge_module.format_token_k(context_usage.last_compaction_before_input_tokens)}"
            f"->{bridge_module.format_token_k(context_usage.last_compaction_after_input_tokens)}."
        )
    return (
        f"Context warning: {context_usage.usage_ratio * 100:.1f}% ({status}), "
        f"archive_recommended={'yes' if archive_recommended else 'no'}, "
        f"token_used_total={bridge_module.format_token_k(thread.tokens_used)}."
        f"{compaction_note} "
        "Use `!context` to inspect, or `!new <prompt>` to continue in a fresh mirrored thread."
    )


def build_context_message(
    channel_id: int | None = None,
    *,
    all_threads: bool = False,
    limit: int = 10,
    bridge_module: ContextStatusBridge,
    get_mirrored_codex_thread_id_func: GetMirroredThreadFunc,
    resolve_selected_target_func: ResolveSelectedTargetFunc,
) -> str:
    if not all_threads:
        target_thread_id = get_mirrored_codex_thread_id_func(channel_id)
        if not target_thread_id:
            selected_thread_id, _target_ref = resolve_selected_target_func()
            target_thread_id = selected_thread_id
        if not target_thread_id:
            return "No Codex thread target found."
        try:
            thread = bridge_module.choose_thread(target_thread_id, None)
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK
            return f"Context unavailable.\n\nERROR: {exc}"
        return "\n".join(
            [
                "Context status",
                f"thread_ref: {bridge_module.get_thread_workspace_ref(thread)}",
                f"title: {bridge_module.get_thread_ui_name(thread.id, thread) or thread.title or '-'}",
                format_context_usage_line(thread, bridge_module=bridge_module),
                f"tokens_used_total: {bridge_module.format_token_k(thread.tokens_used)}",
            ]
        )

    threads = bridge_module.load_recent_threads(limit=max(1, min(50, limit)))
    lines = ["Context status"]
    for thread in threads:
        title = bridge_module.get_thread_ui_name(thread.id, thread) or thread.title or thread.id[:8]
        lines.append(
            "".join(
                [
                    f"- {bridge_module.get_thread_workspace_ref(thread)} / {title}: ",
                    f"{format_context_usage_line(thread, bridge_module=bridge_module)}; ",
                    f"total={bridge_module.format_token_k(thread.tokens_used)}",
                ]
            )
        )
    return "\n".join(lines)
