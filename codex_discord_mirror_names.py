from __future__ import annotations

import hashlib
from collections.abc import Sequence
from typing import Callable, Protocol, TypeVar

from codex_discord_text import normalize_discord_name, truncate_discord_title


class ChannelLike(Protocol):
    @property
    def id(self) -> int: ...

    @property
    def name(self) -> str: ...


class GuildLike(Protocol):
    @property
    def text_channels(self) -> Sequence[ChannelLike]: ...


class ThreadLike(Protocol):
    @property
    def id(self) -> str: ...

    @property
    def title(self) -> str: ...


ThreadT = TypeVar("ThreadT", bound=ThreadLike)


def get_mirror_project_channel_name(
    guild: GuildLike,
    project_key: str,
    project_name: str,
    *,
    current_channel_id: int | None = None,
) -> str:
    base_name = normalize_discord_name(project_name, prefix="codex-", max_len=80)
    existing_names = {
        str(channel.name or "")
        for channel in guild.text_channels
        if current_channel_id is None or int(channel.id or 0) != current_channel_id
    }
    if base_name not in existing_names:
        return base_name
    digest = hashlib.sha1(project_key.encode("utf-8", errors="ignore")).hexdigest()[:6]
    return normalize_discord_name(f"{base_name}-{digest}", max_len=90)


def get_mirror_thread_name(
    codex_thread: ThreadT,
    *,
    get_thread_ui_name: Callable[[str, ThreadT], str],
) -> str:
    title = get_thread_ui_name(codex_thread.id, codex_thread) or codex_thread.title
    return truncate_discord_title(title, f"codex-{codex_thread.id[:8]}", max_len=90)
