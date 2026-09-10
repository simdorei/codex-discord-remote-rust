"""Shared ordinary user-root scope for Discord command surfaces."""

from __future__ import annotations

from collections.abc import Callable
from typing import TypeAlias

from codex_thread_models import ThreadInfo


LoadUserRootThreads: TypeAlias = Callable[[int], list[ThreadInfo]]


def load_ordinary_user_root_threads(
    load_user_root_threads: LoadUserRootThreads,
    *,
    limit: int = 0,
) -> list[ThreadInfo]:
    threads = load_user_root_threads(0)
    if limit > 0:
        return threads[:limit]
    return threads
