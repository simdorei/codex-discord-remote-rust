from __future__ import annotations

from collections.abc import Callable, Sequence
from typing import Protocol


class ArchiveTargetThread(Protocol):
    @property
    def id(self) -> str: ...


class ArchiveTargetBridge(Protocol):
    def load_user_root_threads(self) -> Sequence[ArchiveTargetThread]: ...


ResolveThreadTargetArgsFunc = Callable[[int | None, str | None], list[str]]


class ArchiveTargetError(RuntimeError):
    pass


def resolve_discord_archive_target_args(
    discord_channel_id: int | None,
    ref: str | None,
    *,
    bridge_module: ArchiveTargetBridge,
    resolve_thread_target_args_func: ResolveThreadTargetArgsFunc,
) -> list[str]:
    normalized = str(ref or "").strip()
    if normalized and normalized.isdigit():
        root_threads = bridge_module.load_user_root_threads()
        index = int(normalized)
        if 1 <= index <= len(root_threads):
            return ["--thread-id", root_threads[index - 1].id]
        raise ArchiveTargetError(f"DB root thread index out of range: {normalized}")
    return resolve_thread_target_args_func(discord_channel_id, ref)
