from __future__ import annotations

import sqlite3
from typing import cast

from codex_discord_store_connection import backup_store_before_migration


LATEST_STORE_SCHEMA_VERSION = 2


class StoreSchemaError(RuntimeError):
    pass


class StoreSchemaVersionError(StoreSchemaError):
    pass


class StoreIntegrityError(StoreSchemaError):
    pass

STORE_SCHEMA_TABLES: tuple[str, ...] = (
    "mirror_projects",
    "mirror_threads",
    "session_mirror_details",
    "busy_choices",
    "persistent_component_claims",
    "discord_processed_messages",
    "codex_session_mirror_offsets",
    "codex_session_mirror_events",
    "codex_turn_queue",
)

STORE_SCHEMA_STATEMENTS: tuple[str, ...] = (
    (
        "CREATE TABLE IF NOT EXISTS mirror_projects ("
        "project_key TEXT PRIMARY KEY, "
        "project_name TEXT NOT NULL, "
        "discord_channel_id INTEGER NOT NULL, "
        "updated_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS mirror_threads ("
        "codex_thread_id TEXT PRIMARY KEY, "
        "project_key TEXT NOT NULL, "
        "thread_title TEXT NOT NULL, "
        "discord_channel_id INTEGER NOT NULL, "
        "discord_thread_id INTEGER NOT NULL, "
        "updated_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS session_mirror_details ("
        "codex_thread_id TEXT PRIMARY KEY, "
        "detail_mode TEXT NOT NULL "
        "CHECK(detail_mode IN ('send', 'all')))"
    ),
    (
        "CREATE TABLE IF NOT EXISTS busy_choices ("
        "choice_id TEXT PRIMARY KEY, "
        "owner_user_id INTEGER NOT NULL, "
        "channel_id INTEGER NOT NULL, "
        "target_thread_id TEXT, "
        "prompt TEXT NOT NULL, "
        "allow_steer INTEGER NOT NULL, "
        "created_at REAL NOT NULL, "
        "expires_at REAL NOT NULL, "
        "claimed_at REAL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS persistent_component_claims ("
        "claim_key TEXT PRIMARY KEY, "
        "created_at REAL NOT NULL, "
        "expires_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS discord_processed_messages ("
        "message_id INTEGER PRIMARY KEY, "
        "seen_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS codex_session_mirror_offsets ("
        "codex_thread_id TEXT PRIMARY KEY, "
        "rollout_path TEXT NOT NULL, "
        "cursor INTEGER NOT NULL, "
        "updated_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS codex_session_mirror_events ("
        "event_digest TEXT PRIMARY KEY, "
        "codex_thread_id TEXT NOT NULL, "
        "created_at REAL NOT NULL)"
    ),
    (
        "CREATE TABLE IF NOT EXISTS codex_turn_queue ("
        "job_id TEXT PRIMARY KEY, "
        "target_thread_id TEXT NOT NULL, "
        "channel_id INTEGER NOT NULL, "
        "owner_user_id INTEGER, "
        "discord_message_id INTEGER, "
        "app_server_generation INTEGER NOT NULL DEFAULT 0, "
        "prompt TEXT NOT NULL, "
        "queued INTEGER NOT NULL, "
        "ack_sent INTEGER NOT NULL, "
        "state TEXT NOT NULL, "
        "attempt_count INTEGER NOT NULL, "
        "turn_id TEXT, "
        "baseline_turn_ids TEXT NOT NULL, "
        "last_error TEXT NOT NULL DEFAULT '', "
        "created_at REAL NOT NULL, "
        "updated_at REAL NOT NULL)"
    ),
    (
        "CREATE UNIQUE INDEX IF NOT EXISTS codex_turn_queue_message_id "
        "ON codex_turn_queue(discord_message_id) WHERE discord_message_id IS NOT NULL"
    ),
    (
        "CREATE INDEX IF NOT EXISTS codex_turn_queue_target_order "
        "ON codex_turn_queue(target_thread_id, created_at, job_id)"
    ),
)


def init_store_schema(conn: sqlite3.Connection) -> None:
    current_version = _get_schema_version(conn)
    if current_version > LATEST_STORE_SCHEMA_VERSION:
        raise StoreSchemaVersionError(
            f"Store schema version {current_version} is newer than supported "
            f"version {LATEST_STORE_SCHEMA_VERSION}."
        )
    if current_version == LATEST_STORE_SCHEMA_VERSION:
        return
    if conn.in_transaction:
        raise StoreSchemaError("Store schema migration requires a connection without an active transaction.")

    _ = backup_store_before_migration(
        conn,
        from_version=current_version,
        to_version=LATEST_STORE_SCHEMA_VERSION,
    )
    try:
        _ = conn.execute("BEGIN IMMEDIATE")
        for version in range(current_version + 1, LATEST_STORE_SCHEMA_VERSION + 1):
            _run_migration(conn, version)
        _ = conn.execute(f"PRAGMA user_version = {LATEST_STORE_SCHEMA_VERSION}")
        assert_store_integrity(conn)
        conn.commit()
    except BaseException:
        conn.rollback()
        raise


def _run_migration(conn: sqlite3.Connection, version: int) -> None:
    if version == 1:
        _create_store_schema(conn)
        return
    if version == 2:
        _migrate_codex_turn_queue(conn)
        return
    raise StoreSchemaVersionError(f"No store migration is registered for version {version}.")


def _create_store_schema(conn: sqlite3.Connection) -> None:
    for statement in STORE_SCHEMA_STATEMENTS:
        _ = conn.execute(statement)


def _get_schema_version(conn: sqlite3.Connection) -> int:
    row = cast(tuple[int] | None, conn.execute("PRAGMA user_version").fetchone())
    if row is None:
        raise StoreSchemaVersionError("Store schema version could not be read.")
    return int(row[0])


def assert_store_integrity(conn: sqlite3.Connection) -> None:
    row = cast(tuple[str] | None, conn.execute("PRAGMA integrity_check").fetchone())
    result = str(row[0]) if row is not None else "missing result"
    if result.lower() != "ok":
        raise StoreIntegrityError(f"SQLite integrity check failed: {result}")


def _migrate_codex_turn_queue(conn: sqlite3.Connection) -> None:
    rows = cast(
        list[tuple[int, str, str, int, object, int]],
        conn.execute("PRAGMA table_info(codex_turn_queue)").fetchall(),
    )
    columns = {
        row[1]
        for row in rows
    }
    if "app_server_generation" not in columns:
        _ = conn.execute(
            "ALTER TABLE codex_turn_queue "
            + "ADD COLUMN app_server_generation INTEGER NOT NULL DEFAULT 0"
        )
