from __future__ import annotations

import re
from typing import Final

from codex_discord_message_mentions import strip_required_plain_ask_mentions

BOT_BRIDGE_OPERATIONAL_PACKET_PREFIXES: Final = (
    "PROGRESS:",
    "ACTION:",
    "ACK:",
    "FINAL:",
    "BLOCKER:",
    "완료:",
    "PRE-RESTART/HANDOFF",
    "RESTART-CHECK/HANDOFF",
)
BOT_BRIDGE_LEADING_MENTION_RE: Final = re.compile(r"^(?:(?:<@!?\d+>|<@\*+>)\s*)+")


def normalize_bot_bridge_packet_content(content: str) -> str:
    return BOT_BRIDGE_LEADING_MENTION_RE.sub("", content.lstrip()).strip()


def is_bot_bridge_operational_packet(content: str, bridge_user_ids: set[int]) -> bool:
    stripped_content, matched_bridge_mention = strip_required_plain_ask_mentions(
        content,
        bridge_user_ids,
    )
    if not matched_bridge_mention:
        return False
    normalized = normalize_bot_bridge_packet_content(stripped_content)
    if is_bot_bridge_restart_check_handoff_packet(normalized):
        return False
    return normalized.startswith(BOT_BRIDGE_OPERATIONAL_PACKET_PREFIXES)


def is_bot_bridge_restart_check_handoff_packet(normalized_content: str) -> bool:
    normalized_upper = normalized_content.upper()
    if (
        not normalized_content.startswith("ACTION:")
        and not normalized_content.startswith("RESTART-CHECK/HANDOFF")
    ):
        return False
    return "RESTART-CHECK" in normalized_upper and "HANDOFF" in normalized_upper
