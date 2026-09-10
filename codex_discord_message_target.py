from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Literal

import codex_discord_explicit_target as discord_explicit_target

MessageTargetSource = Literal["mirror", "selected", "explicit"]
MirrorThreadLookup = Callable[[int | None], str | None]


@dataclass(frozen=True, slots=True)
class DiscordMessageTarget:
    target_thread_id: str | None
    target_source: MessageTargetSource
    persist_mirror_channel: bool = False

    def with_explicit_target(self, content: str, *, bot_bridge_mention: bool) -> DiscordMessageTarget:
        if not bot_bridge_mention:
            return self
        explicit_target_thread_id = discord_explicit_target.extract_explicit_codex_thread_id(content)
        if explicit_target_thread_id is None:
            return self
        return DiscordMessageTarget(
            target_thread_id=explicit_target_thread_id,
            target_source="explicit",
        )


def resolve_discord_message_target(
    lookup_mirrored_codex_thread_id: MirrorThreadLookup,
    channel_id: int | None,
    parent_channel_id: int | None,
) -> DiscordMessageTarget:
    target_thread_id = lookup_mirrored_codex_thread_id(channel_id)
    if target_thread_id is not None:
        return DiscordMessageTarget(
            target_thread_id=target_thread_id,
            target_source="mirror",
            persist_mirror_channel=parent_channel_id is not None,
        )
    target_thread_id = lookup_mirrored_codex_thread_id(parent_channel_id)
    target_source: MessageTargetSource = "mirror" if target_thread_id else "selected"
    return DiscordMessageTarget(
        target_thread_id=target_thread_id,
        target_source=target_source,
    )
