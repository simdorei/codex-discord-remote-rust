from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import json
import os
import re
import secrets
import sqlite3
import sys
import time
from pathlib import Path
from typing import Final, Literal, assert_never, cast
from urllib.parse import urlsplit


SCOPE_PATTERN: Final = re.compile(r"^codex-pro-[a-f0-9]{24}$")
CHATGPT_HOSTS: Final = frozenset({"chatgpt.com", "www.chatgpt.com"})
LEASE_SECONDS: Final = 120


class ConversationMapError(Exception):
    """Raised when a conversation mapping command is invalid."""


class _Arguments(argparse.Namespace):
    command: Literal[
        "acquire",
        "set",
        "release",
        "delete",
        "restart",
        "status",
        "complete-restart",
        "restore-stalled",
    ] | None = None
    scope: str | None = None
    url: str | None = None
    failed_url: str | None = None
    lease_token: str | None = None


_AcquireRow = tuple[str | None, str | None, int | None, int]
_SaveRow = tuple[str | None, int, str | None]
_ReleaseRow = tuple[str | None, str | None, int, str | None]
_CompleteRow = tuple[str | None, int]
_RestoreRow = tuple[str | None, str | None, int | None, int, str | None]


def _row_values(value: object, expected: int) -> tuple[object, ...] | None:
    if value is None:
        return None
    if not isinstance(value, tuple):
        raise ConversationMapError("The conversation store contains invalid data.")
    row = cast(tuple[object, ...], value)
    if len(row) != expected:
        raise ConversationMapError("The conversation store contains invalid data.")
    return row


def _optional_text(value: object) -> str | None:
    if value is None or isinstance(value, str):
        return value
    raise ConversationMapError("The conversation store contains invalid data.")


def _optional_integer(value: object) -> int | None:
    if value is None:
        return None
    if isinstance(value, int) and not isinstance(value, bool):
        return value
    raise ConversationMapError("The conversation store contains invalid data.")


def _flag(value: object) -> int:
    if isinstance(value, int) and not isinstance(value, bool) and value in (0, 1):
        return value
    raise ConversationMapError("The conversation store contains invalid data.")


def _acquire_row(value: object) -> _AcquireRow | None:
    row = _row_values(value, 4)
    if row is None:
        return None
    return (
        _optional_text(row[0]),
        _optional_text(row[1]),
        _optional_integer(row[2]),
        _flag(row[3]),
    )


def _save_row(value: object) -> _SaveRow | None:
    row = _row_values(value, 3)
    if row is None:
        return None
    lease_hash = _optional_text(row[0])
    restart_pending = _flag(row[1])
    restart_from_url = _optional_text(row[2])
    if bool(restart_pending) != (restart_from_url is not None):
        raise ConversationMapError("The conversation store contains invalid data.")
    return (lease_hash, restart_pending, restart_from_url)


def _release_row(value: object) -> _ReleaseRow | None:
    row = _row_values(value, 4)
    if row is None:
        return None
    return (
        _optional_text(row[0]),
        _optional_text(row[1]),
        _flag(row[2]),
        _optional_text(row[3]),
    )


def _complete_row(value: object) -> _CompleteRow | None:
    row = _row_values(value, 2)
    if row is None:
        return None
    return (_optional_text(row[0]), _flag(row[1]))


def _restore_row(value: object) -> _RestoreRow | None:
    row = _row_values(value, 5)
    if row is None:
        return None
    return (
        _optional_text(row[0]),
        _optional_text(row[1]),
        _optional_integer(row[2]),
        _flag(row[3]),
        _optional_text(row[4]),
    )


def database_path() -> Path:
    override = os.environ.get("SIMDOREI_PRO_CONVERSATION_DB", "").strip()
    if override:
        return Path(override).expanduser().resolve()
    local_data = os.environ.get("LOCALAPPDATA", "").strip()
    if local_data:
        base = Path(local_data)
    else:
        state_home = os.environ.get("XDG_STATE_HOME", "").strip()
        base = Path(state_home) if state_home else Path.home() / ".local" / "state"
    return base / "simdorei" / "ask-chatgpt-pro" / "conversations.sqlite3"


def acquire(scope: str) -> dict[str, str]:
    _validate_scope(scope)
    now = int(time.time())
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, lease_hash, lease_expires_at,
                       restart_pending
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _acquire_row(raw_row)
        conversation_url = row[0] if row is not None else None
        if conversation_url:
            _validate_url(conversation_url)
            connection.commit()
            return {"status": "found", "url": str(conversation_url)}
        if row is not None and row[1] and (row[2] or 0) >= now:
            connection.commit()
            return {"status": "busy"}
        if row is not None and row[3]:
            connection.commit()
            return {"status": "stalled"}
        lease_token = "lease_" + secrets.token_urlsafe(24)
        _ = connection.execute(
            """
            INSERT INTO conversations(
                scope, conversation_url, lease_hash, lease_expires_at, updated_at
            ) VALUES (?, NULL, ?, ?, ?)
            ON CONFLICT(scope) DO UPDATE SET
                lease_hash = excluded.lease_hash,
                lease_expires_at = excluded.lease_expires_at,
                updated_at = excluded.updated_at
            """,
            (
                scope,
                _token_hash(lease_token),
                now + LEASE_SECONDS,
                now,
            ),
        )
        connection.commit()
    return {"status": "acquired", "lease_token": lease_token}


def status(scope: str) -> dict[str, str]:
    _validate_scope(scope)
    now = int(time.time())
    with closing(_connect()) as connection:
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, lease_hash, lease_expires_at,
                       restart_pending
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
    row = _acquire_row(raw_row)
    if row is None:
        return {"status": "missing"}
    conversation_url, lease_hash, lease_expires_at, restart_pending = row
    if conversation_url:
        _validate_url(conversation_url)
        return {"status": "found", "url": conversation_url}
    if lease_hash and (lease_expires_at or 0) >= now:
        return {"status": "busy"}
    if restart_pending:
        return {"status": "stalled"}
    return {"status": "missing"}


def restart(scope: str, failed_url: str) -> dict[str, str]:
    """Atomically replace the mapping for one confirmed failed conversation."""
    _validate_scope(scope)
    _validate_url(failed_url)
    now = int(time.time())
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, lease_hash, lease_expires_at,
                       restart_pending
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _acquire_row(raw_row)
        if row is None:
            connection.commit()
            return {"status": "missing"}
        conversation_url, lease_hash, lease_expires_at, restart_pending = row
        if conversation_url:
            _validate_url(conversation_url)
        if restart_pending:
            if conversation_url and _same_conversation(conversation_url, failed_url):
                connection.commit()
                return {"status": "exhausted", "url": conversation_url}
            if conversation_url:
                connection.commit()
                return {"status": "superseded", "url": conversation_url}
            if lease_hash and (lease_expires_at or 0) >= now:
                connection.commit()
                return {"status": "busy"}
            connection.commit()
            return {"status": "stalled"}
        if conversation_url and not _same_conversation(conversation_url, failed_url):
            connection.commit()
            return {"status": "superseded", "url": conversation_url}
        if conversation_url is None:
            if lease_hash and (lease_expires_at or 0) >= now:
                connection.commit()
                return {"status": "busy"}
            connection.commit()
            return {"status": "missing"}

        lease_token = "lease_" + secrets.token_urlsafe(24)
        _ = connection.execute(
            """
            UPDATE conversations
            SET conversation_url = NULL, lease_hash = ?,
                lease_expires_at = ?, updated_at = ?,
                restart_pending = 1, restart_from_url = ?
            WHERE scope = ? AND conversation_url = ?
            """,
            (
                _token_hash(lease_token),
                now + LEASE_SECONDS,
                now,
                conversation_url,
                scope,
                conversation_url,
            ),
        )
        connection.commit()
    return {"status": "acquired", "lease_token": lease_token}


def save(scope: str, url: str, lease_token: str) -> dict[str, str]:
    _validate_scope(scope)
    _validate_url(url)
    _validate_lease_token(lease_token)
    now = int(time.time())
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT lease_hash, restart_pending, restart_from_url
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _save_row(raw_row)
        # Expiry is a stale-work signal. A newer lease or guarded restore changes
        # the hash and is the operation that actually fences the prior owner.
        if row is None or not secrets.compare_digest(
            str(row[0] or ""),
            _token_hash(lease_token),
        ):
            raise ConversationMapError(
                "The conversation creation lease is missing or was replaced."
            )
        if row[1] and row[2] and _same_conversation(url, row[2]):
            raise ConversationMapError(
                "The replacement conversation must differ from the failed conversation."
            )
        _ = connection.execute(
            """
            UPDATE conversations
            SET conversation_url = ?, lease_hash = NULL,
                lease_expires_at = NULL, updated_at = ?
            WHERE scope = ?
            """,
            (url, now, scope),
        )
        connection.commit()
    return {"status": "saved"}


def release(scope: str, lease_token: str) -> dict[str, str]:
    _validate_scope(scope)
    _validate_lease_token(lease_token)
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT lease_hash, conversation_url, restart_pending,
                       restart_from_url
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _release_row(raw_row)
        # An expired but still-matching token remains the owner until a fencing
        # transition replaces or clears its hash.
        if row is not None and secrets.compare_digest(
            str(row[0] or ""),
            _token_hash(lease_token),
        ):
            if row[1]:
                _ = connection.execute(
                    """
                    UPDATE conversations
                    SET lease_hash = NULL, lease_expires_at = NULL
                    WHERE scope = ?
                    """,
                    (scope,),
                )
            elif row[2] and row[3]:
                _validate_url(row[3])
                _ = connection.execute(
                    """
                    UPDATE conversations
                    SET conversation_url = ?, lease_hash = NULL,
                        lease_expires_at = NULL, updated_at = ?,
                        restart_pending = 0, restart_from_url = NULL
                    WHERE scope = ?
                    """,
                    (row[3], int(time.time()), scope),
                )
            else:
                _ = connection.execute(
                    "DELETE FROM conversations WHERE scope = ?",
                    (scope,),
                )
        connection.commit()
    return {"status": "released"}


def complete_restart(scope: str, url: str) -> dict[str, str]:
    _validate_scope(scope)
    _validate_url(url)
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, restart_pending
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _complete_row(raw_row)
        if row is None:
            connection.commit()
            return {"status": "missing"}
        conversation_url, restart_pending = row
        if conversation_url is None:
            connection.commit()
            return {"status": "unavailable"}
        _validate_url(conversation_url)
        if not _same_conversation(conversation_url, url):
            connection.commit()
            return {"status": "superseded", "url": conversation_url}
        if not restart_pending:
            connection.commit()
            return {"status": "unchanged"}
        _ = connection.execute(
            """
            UPDATE conversations
            SET restart_pending = 0, restart_from_url = NULL,
                updated_at = ?
            WHERE scope = ?
            """,
            (int(time.time()), scope),
        )
        connection.commit()
    return {"status": "completed"}


def restore_stalled(scope: str, failed_url: str) -> dict[str, str]:
    """Restore the failed mapping after an operator verifies recovery is abandoned."""
    _validate_scope(scope)
    _validate_url(failed_url)
    now = int(time.time())
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, lease_hash, lease_expires_at,
                       restart_pending, restart_from_url
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _restore_row(raw_row)
        if row is None:
            connection.commit()
            return {"status": "missing"}
        conversation_url, lease_hash, lease_expires_at, restart_pending, origin = row
        if conversation_url:
            _validate_url(conversation_url)
            connection.commit()
            return {"status": "superseded", "url": conversation_url}
        if not restart_pending or origin is None:
            connection.commit()
            return {"status": "missing"}
        _validate_url(origin)
        if not _same_conversation(origin, failed_url):
            connection.commit()
            return {"status": "superseded", "url": origin}
        if lease_hash and (lease_expires_at or 0) >= now:
            connection.commit()
            return {"status": "busy"}
        _ = connection.execute(
            """
            UPDATE conversations
            SET conversation_url = ?, lease_hash = NULL,
                lease_expires_at = NULL, updated_at = ?,
                restart_pending = 0, restart_from_url = NULL
            WHERE scope = ?
            """,
            (origin, now, scope),
        )
        connection.commit()
    return {"status": "restored", "url": origin}


def delete(scope: str) -> dict[str, str]:
    _validate_scope(scope)
    with closing(_connect()) as connection:
        _ = connection.execute("BEGIN IMMEDIATE")
        raw_row = cast(
            object,
            connection.execute(
                """
                SELECT conversation_url, restart_pending
                FROM conversations
                WHERE scope = ?
                """,
                (scope,),
            ).fetchone(),
        )
        row = _complete_row(raw_row)
        if row is not None and row[1]:
            conversation_url = row[0]
            connection.commit()
            if conversation_url:
                _validate_url(conversation_url)
                return {"status": "protected", "url": conversation_url}
            return {"status": "protected"}
        _ = connection.execute(
            "DELETE FROM conversations WHERE scope = ?",
            (scope,),
        )
        connection.commit()
    return {"status": "deleted"}


def _connect() -> sqlite3.Connection:
    path = database_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(path, timeout=10)
    try:
        _ = connection.execute("BEGIN IMMEDIATE")
        _ = connection.execute(
            """
            CREATE TABLE IF NOT EXISTS conversations (
                scope TEXT PRIMARY KEY,
                conversation_url TEXT,
                lease_hash TEXT,
                lease_expires_at INTEGER,
                updated_at INTEGER NOT NULL,
                restart_pending INTEGER NOT NULL DEFAULT 0,
                restart_from_url TEXT
            )
            """
        )
        columns = {
            str(row[1])
            for row in connection.execute("PRAGMA table_info(conversations)")
        }
        if "restart_pending" not in columns:
            _ = connection.execute(
                """
                ALTER TABLE conversations
                ADD COLUMN restart_pending INTEGER NOT NULL DEFAULT 0
                """
            )
        if "restart_from_url" not in columns:
            _ = connection.execute(
                "ALTER TABLE conversations ADD COLUMN restart_from_url TEXT"
            )
        connection.commit()
    except Exception:
        connection.rollback()
        connection.close()
        raise
    return connection


def _validate_scope(scope: str) -> None:
    if SCOPE_PATTERN.fullmatch(scope) is None:
        raise ConversationMapError("The conversation scope is invalid.")


def _validate_url(url: str) -> None:
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.hostname not in CHATGPT_HOSTS
        or parsed.username is not None
        or parsed.password is not None
        or "/c/" not in parsed.path
    ):
        raise ConversationMapError(
            "Only canonical HTTPS chatgpt.com conversation URLs are allowed."
        )


def _same_conversation(left: str, right: str) -> bool:
    _validate_url(left)
    _validate_url(right)
    left_parts = urlsplit(left)
    right_parts = urlsplit(right)
    left_host = str(left_parts.hostname).removeprefix("www.")
    right_host = str(right_parts.hostname).removeprefix("www.")
    return (left_host, left_parts.path.rstrip("/")) == (
        right_host,
        right_parts.path.rstrip("/"),
    )


def _validate_lease_token(lease_token: str) -> None:
    if len(lease_token) < 24:
        raise ConversationMapError("The conversation creation lease is invalid.")


def _token_hash(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("acquire", "delete", "status"):
        command = commands.add_parser(name)
        _ = command.add_argument("--scope", required=True)
    save_command = commands.add_parser("set")
    _ = save_command.add_argument("--scope", required=True)
    _ = save_command.add_argument("--url", required=True)
    _ = save_command.add_argument("--lease-token", required=True)
    release_command = commands.add_parser("release")
    _ = release_command.add_argument("--scope", required=True)
    _ = release_command.add_argument("--lease-token", required=True)
    restart_command = commands.add_parser("restart")
    _ = restart_command.add_argument("--scope", required=True)
    _ = restart_command.add_argument("--failed-url", required=True)
    complete_command = commands.add_parser("complete-restart")
    _ = complete_command.add_argument("--scope", required=True)
    _ = complete_command.add_argument("--url", required=True)
    restore_command = commands.add_parser("restore-stalled")
    _ = restore_command.add_argument("--scope", required=True)
    _ = restore_command.add_argument("--failed-url", required=True)
    return parser


def main() -> int:
    arguments = _parser().parse_args(namespace=_Arguments())
    try:
        if arguments.command is None or arguments.scope is None:
            raise ConversationMapError("The command arguments are incomplete.")
        match arguments.command:
            case "acquire":
                result = acquire(arguments.scope)
            case "set":
                if arguments.url is None or arguments.lease_token is None:
                    raise ConversationMapError(
                        "The set command arguments are incomplete."
                    )
                result = save(
                    arguments.scope,
                    arguments.url,
                    arguments.lease_token,
                )
            case "release":
                if arguments.lease_token is None:
                    raise ConversationMapError(
                        "The release command arguments are incomplete."
                    )
                result = release(arguments.scope, arguments.lease_token)
            case "delete":
                result = delete(arguments.scope)
            case "restart":
                if arguments.failed_url is None:
                    raise ConversationMapError(
                        "The restart command arguments are incomplete."
                    )
                result = restart(arguments.scope, arguments.failed_url)
            case "status":
                result = status(arguments.scope)
            case "complete-restart":
                if arguments.url is None:
                    raise ConversationMapError(
                        "The complete-restart command arguments are incomplete."
                    )
                result = complete_restart(arguments.scope, arguments.url)
            case "restore-stalled":
                if arguments.failed_url is None:
                    raise ConversationMapError(
                        "The restore-stalled command arguments are incomplete."
                    )
                result = restore_stalled(arguments.scope, arguments.failed_url)
            case _:
                assert_never(arguments.command)
    except ConversationMapError as exc:
        print(str(exc), file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
