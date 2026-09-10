from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Protocol, cast

import codex_discord_bot as bot
from codex_thread_models import ThreadInfo


class MirrorSyncBridge(Protocol):
    CODEX_HOME: Path
    STATE_DB_PATH: Path
    load_recent_threads: Callable[[int], list[ThreadInfo]]
    load_user_root_threads: Callable[[int], list[ThreadInfo]]
    is_codex_desktop_window_title: Callable[[str], bool]


def bridge_module() -> MirrorSyncBridge:
    return cast(MirrorSyncBridge, vars(bot)["bridge"])


def codex_discord_bot(value: object) -> bot.CodexDiscordBot:
    return cast(bot.CodexDiscordBot, value)
