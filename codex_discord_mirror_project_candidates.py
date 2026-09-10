from __future__ import annotations

from collections.abc import Sequence
from typing import Protocol

import discord


class ProjectChannelCandidate(Protocol):
    @property
    def id(self) -> int: ...

    @property
    def name(self) -> str: ...

    @property
    def topic(self) -> str | None: ...


class ProjectGuildLike(Protocol):
    @property
    def text_channels(self) -> Sequence[ProjectChannelCandidate]: ...


def find_existing_project_channel(
    guild: ProjectGuildLike,
    *,
    project_name: str,
    base_name: str,
) -> discord.TextChannel | None:
    expected_topic = f"Codex project mirror: {project_name}"
    for channel in guild.text_channels:
        if not isinstance(channel, discord.TextChannel):
            continue
        topic = str(getattr(channel, "topic", "") or "")
        name = str(getattr(channel, "name", "") or "")
        if topic == expected_topic:
            return channel
        if name == base_name and topic.startswith("Codex project mirror:"):
            return channel
    return None
