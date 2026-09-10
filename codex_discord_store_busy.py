from __future__ import annotations

import hashlib
import os
import sqlite3
import time
from pathlib import Path
from typing import Protocol, cast

from codex_discord_components import get_persistent_component_claim_key
from codex_discord_store_connection import connect_store
from codex_discord_store_schema import init_store_schema

_DiscordIdValue = int | str | bytes | bytearray
_BusyChoiceRecordValue = str | int | bool | float | None


class _DiscordIdLike(Protocol):
    @property
    def id(self) -> _DiscordIdValue: ...


class _BusyChoiceMessageLike(Protocol):
    @property
    def author(self) -> _DiscordIdLike: ...

    @property
    def channel(self) -> _DiscordIdLike: ...


class _PersistentComponentMessageLike(Protocol):
    @property
    def id(self) -> _DiscordIdValue: ...


class _PersistentComponentInteractionLike(Protocol):
    @property
    def message(self) -> _PersistentComponentMessageLike | None: ...


def _init_mirror_db(db_path: Path) -> None:
    with connect_store(db_path) as conn:
        init_store_schema(conn)


def _coerce_discord_id(value: _DiscordIdValue) -> int:
    return int(value)


def cleanup_expired_busy_choices(db_path: Path, now: float | None = None) -> int:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        result = conn.execute(
            "DELETE FROM busy_choices WHERE expires_at <= ? OR claimed_at IS NOT NULL",
            (current,),
        )
        return result.rowcount


def cleanup_expired_persistent_component_claims(db_path: Path, now: float | None = None) -> int:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        result = conn.execute(
            "DELETE FROM persistent_component_claims WHERE expires_at <= ?",
            (current,),
        )
        return result.rowcount


def create_busy_choice_record(
    db_path: Path,
    message: _BusyChoiceMessageLike,
    prompt: str,
    target_thread_id: str | None,
    *,
    allow_steer: bool,
    ttl_seconds: float,
) -> str:
    _ = cleanup_expired_busy_choices(db_path)
    now = time.time()
    choice_id = hashlib.sha256(
        f"{now}:{os.urandom(16).hex()}:{message.author.id}".encode("utf-8")
    ).hexdigest()[:24]
    with connect_store(db_path) as conn:
        _ = conn.execute(
            "INSERT INTO busy_choices ("
            + "choice_id, owner_user_id, channel_id, target_thread_id, prompt, "
            + "allow_steer, created_at, expires_at, claimed_at"
            + ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL)",
            (
                choice_id,
                _coerce_discord_id(message.author.id),
                _coerce_discord_id(message.channel.id),
                target_thread_id,
                str(prompt or ""),
                1 if allow_steer else 0,
                now,
                now + ttl_seconds,
            ),
        )
    return choice_id


def get_busy_choice_record(
    db_path: Path,
    choice_id: str,
) -> dict[str, _BusyChoiceRecordValue] | None:
    _init_mirror_db(db_path)
    now = time.time()
    with connect_store(db_path) as conn:
        row = cast(
            tuple[int, int, str | None, str | None, int, float, float, float | None] | None,
            conn.execute(
                "SELECT owner_user_id, channel_id, target_thread_id, prompt, "
                + "allow_steer, created_at, expires_at, claimed_at "
                + "FROM busy_choices WHERE choice_id = ?",
                (choice_id,),
            ).fetchone(),
        )
        if not row:
            return None
        if float(row[6]) <= now or row[7] is not None:
            _ = conn.execute("DELETE FROM busy_choices WHERE choice_id = ?", (choice_id,))
            return None
    return {
        "choice_id": choice_id,
        "owner_user_id": int(row[0]),
        "channel_id": int(row[1]),
        "target_thread_id": str(row[2] or "") or None,
        "prompt": str(row[3] or ""),
        "allow_steer": bool(row[4]),
        "created_at": float(row[5]),
        "expires_at": float(row[6]),
    }


def claim_busy_choice_record(db_path: Path, choice_id: str) -> bool:
    _init_mirror_db(db_path)
    now = time.time()
    with connect_store(db_path) as conn:
        result = conn.execute(
            "UPDATE busy_choices "
            + "SET claimed_at = ? "
            + "WHERE choice_id = ? AND claimed_at IS NULL AND expires_at > ?",
            (now, choice_id, now),
        )
        return result.rowcount == 1


def claim_persistent_component_interaction(
    db_path: Path,
    interaction: _PersistentComponentInteractionLike,
    custom_id: str,
    *,
    ttl_seconds: float = 86400.0,
) -> bool:
    claim_key = get_persistent_component_claim_key(interaction, custom_id)
    if claim_key is None:
        return True
    _init_mirror_db(db_path)
    now = time.time()
    with connect_store(db_path) as conn:
        _ = conn.execute("DELETE FROM persistent_component_claims WHERE expires_at <= ?", (now,))
        result = conn.execute(
            "INSERT OR IGNORE INTO persistent_component_claims "
            + "(claim_key, created_at, expires_at) VALUES (?, ?, ?)",
            (claim_key, now, now + ttl_seconds),
        )
        return result.rowcount == 1


def get_busy_choice_counts(db_path: Path, now: float | None = None) -> tuple[int, int]:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        active = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT COUNT(*) FROM busy_choices "
                + "WHERE expires_at > ? AND claimed_at IS NULL",
                (current,),
            ).fetchone(),
        )
        stale = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT COUNT(*) FROM busy_choices "
                + "WHERE expires_at <= ? OR claimed_at IS NOT NULL",
                (current,),
            ).fetchone(),
        )
    return int(active[0] if active else 0), int(stale[0] if stale else 0)


def get_persistent_component_claim_counts(
    db_path: Path,
    now: float | None = None,
) -> tuple[int, int]:
    current = time.time() if now is None else now
    _init_mirror_db(db_path)
    with connect_store(db_path) as conn:
        active = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT COUNT(*) FROM persistent_component_claims WHERE expires_at > ?",
                (current,),
            ).fetchone(),
        )
        stale = cast(
            tuple[int] | None,
            conn.execute(
                "SELECT COUNT(*) FROM persistent_component_claims WHERE expires_at <= ?",
                (current,),
            ).fetchone(),
        )
    return int(active[0] if active else 0), int(stale[0] if stale else 0)
