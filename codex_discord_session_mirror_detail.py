from __future__ import annotations

from enum import StrEnum


class SessionMirrorDetailMode(StrEnum):
    SEND = "send"
    ALL = "all"


def parse_session_mirror_detail_mode(
    raw_value: str,
) -> SessionMirrorDetailMode | None:
    try:
        return SessionMirrorDetailMode(raw_value.strip().lower())
    except ValueError:
        return None
