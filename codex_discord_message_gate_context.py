from __future__ import annotations

import os
from typing import Final

from codex_discord_text import env_flag

DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS: Final = (
    "codex,코덱스,bridge,브릿지,discord,디스코드,디코,bot,봇,응답,"
    + "message,메시지,메세지,채팅,thread,스레드,queue,큐,steer,스티어,"
    + "patch,패치,qa,하네스,harness,잘아타스"
)


def plain_ask_context_fallback_enabled() -> bool:
    return env_flag("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", default=False)


def get_plain_ask_context_keywords() -> list[str]:
    raw = os.environ.get(
        "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
        DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
    )
    return [part.strip().lower() for part in raw.split(",") if part.strip()]


def plain_ask_context_matches(content: str) -> bool:
    normalized = content.lower()
    return any(keyword in normalized for keyword in get_plain_ask_context_keywords())


def should_accept_plain_ask_without_required_mention(content: str) -> bool:
    if not plain_ask_context_fallback_enabled():
        return False
    return plain_ask_context_matches(content)
