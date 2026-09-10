from __future__ import annotations

from collections.abc import Callable
from typing import Protocol


class QueueTargetThread(Protocol):
    @property
    def id(self) -> str: ...


class QueueTargetBridge(Protocol):
    def resolve_thread_ref(self, ref: str) -> QueueTargetThread: ...


ResolveTargetRefFunc = Callable[[str | None], tuple[str | None, str]]
GetMirroredCodexThreadIdFunc = Callable[[int | None], str | None]


def resolve_queue_command_target(
    channel_id: int | None,
    ref: str | None,
    *,
    bridge_module: QueueTargetBridge,
    resolve_target_ref_func: ResolveTargetRefFunc,
    get_mirrored_codex_thread_id_func: GetMirroredCodexThreadIdFunc,
) -> tuple[str | None, str]:
    normalized = str(ref or "").strip()
    if normalized:
        if normalized.lower() in {"selected", "current"}:
            return None, "selected"
        thread = bridge_module.resolve_thread_ref(normalized)
        _resolved_thread_id, target_ref = resolve_target_ref_func(thread.id)
        return thread.id, target_ref or normalized
    target_thread_id = get_mirrored_codex_thread_id_func(channel_id)
    if target_thread_id:
        _resolved_thread_id, target_ref = resolve_target_ref_func(target_thread_id)
        return target_thread_id, target_ref or target_thread_id
    return None, "selected"
