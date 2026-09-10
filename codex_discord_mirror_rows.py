from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass

import codex_discord_mirror_access as mirror_access

type SqliteMirrorRow = Mapping[str, str | int | float | None]


class MirrorRowError(RuntimeError):
    pass


class MirrorListRowError(MirrorRowError):
    pass


class MirrorCheckRowError(MirrorRowError):
    pass


@dataclass(frozen=True, slots=True)
class MirrorListRow:
    title: str
    codex_thread_id: str
    project_name: str
    discord_thread_id: int
    parent_channel_id: int = 0
    accessible: str = mirror_access.ACCESS_UNKNOWN
    archived: str = mirror_access.ARCHIVED_UNKNOWN
    last_seen: float = 0.0
    stale: bool = False
    reason: str = mirror_access.ACTIVE_MAPPING_REASON


@dataclass(frozen=True, slots=True)
class MirrorCheckRow:
    codex_thread_id: str
    project_key: str
    discord_thread_id: int
    parent_channel_id: int = 0
    accessible: str = mirror_access.ACCESS_UNKNOWN
    archived: str = mirror_access.ARCHIVED_UNKNOWN
    last_seen: float = 0.0
    stale: bool = False
    reason: str = mirror_access.ACTIVE_MAPPING_REASON


def _optional_row_value(row: SqliteMirrorRow, key: str) -> str | int | float | None:
    try:
        return row[key]
    except (KeyError, IndexError):
        return None


def mirror_list_row_from_db(row: SqliteMirrorRow) -> MirrorListRow:
    raw_discord_thread_id = row["discord_thread_id"]
    if raw_discord_thread_id is None:
        raise MirrorListRowError("Mirror list row is missing discord_thread_id.")
    return MirrorListRow(
        title=str(row["thread_title"] or ""),
        codex_thread_id=str(row["codex_thread_id"] or ""),
        project_name=str(row["project_name"] or ""),
        discord_thread_id=int(raw_discord_thread_id),
        parent_channel_id=int(_optional_row_value(row, "parent_channel_id") or 0),
        last_seen=float(_optional_row_value(row, "last_seen") or 0.0),
    )


def mirror_check_row_from_db(row: SqliteMirrorRow) -> MirrorCheckRow:
    raw_discord_thread_id = row["discord_thread_id"]
    if raw_discord_thread_id is None:
        raise MirrorCheckRowError("Mirror check row is missing discord_thread_id.")
    return MirrorCheckRow(
        codex_thread_id=str(row["codex_thread_id"] or ""),
        project_key=str(row["project_key"] or ""),
        discord_thread_id=int(raw_discord_thread_id),
        parent_channel_id=int(_optional_row_value(row, "parent_channel_id") or 0),
        last_seen=float(_optional_row_value(row, "last_seen") or 0.0),
    )
