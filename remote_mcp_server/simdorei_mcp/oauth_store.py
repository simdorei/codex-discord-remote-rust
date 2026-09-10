"""Cohesive SQLite storage for OAuth clients and token families. (# noqa: SIZE_OK)"""

from __future__ import annotations

import hashlib
import json
import sqlite3
import threading
import time
from collections.abc import Generator
from contextlib import contextmanager
from enum import Enum, auto
from pathlib import Path

from anyio import to_thread
from mcp.server.auth.provider import AccessToken, RefreshToken
from mcp.shared.auth import OAuthClientInformationFull


class OAuthStoreConfigurationError(ValueError):
    """Raised when OAuth storage is configured with invalid bounds."""


class OAuthClientLimitError(Exception):
    """Raised when every retained OAuth client still has a live token."""


class OAuthTokenFamilyLimitError(Exception):
    """Raised when issuing another live OAuth token family would exceed a cap."""


class RefreshRotationOutcome(Enum):
    ROTATED = auto()
    MISSING = auto()
    REPLAYED = auto()
    HISTORY_EXHAUSTED = auto()


class OAuthStore:
    """Small persistent store for one-user OAuth clients and hashed tokens."""

    def __init__(
        self,
        database_path: Path,
        *,
        max_clients: int = 500,
        max_token_families: int = 256,
        max_token_families_per_client: int = 16,
        max_refresh_history_global: int = 65_536,
        max_refresh_history_per_family: int = 1_024,
    ) -> None:
        if max_clients < 1:
            raise OAuthStoreConfigurationError("OAuth client limit must be positive.")
        if max_token_families < 1 or max_token_families_per_client < 1:
            raise OAuthStoreConfigurationError(
                "OAuth token family limits must be positive."
            )
        if max_token_families_per_client > max_token_families:
            raise OAuthStoreConfigurationError(
                "The per-client token family limit cannot exceed the global limit."
            )
        if max_refresh_history_global < 1 or max_refresh_history_per_family < 1:
            raise OAuthStoreConfigurationError(
                "OAuth refresh history limits must be positive."
            )
        if max_refresh_history_per_family > max_refresh_history_global:
            raise OAuthStoreConfigurationError(
                "The per-family refresh history limit cannot exceed the global limit."
            )
        if str(database_path) != ":memory:":
            database_path.parent.mkdir(parents=True, exist_ok=True)
        self._connection = sqlite3.connect(
            str(database_path),
            check_same_thread=False,
        )
        self._connection.row_factory = sqlite3.Row
        self._lock = threading.RLock()
        self._max_clients = max_clients
        self._max_token_families = max_token_families
        self._max_token_families_per_client = max_token_families_per_client
        self._max_refresh_history_global: int = max_refresh_history_global
        self._max_refresh_history_per_family: int = max_refresh_history_per_family
        self._initialize()

    async def close(self) -> None:
        await to_thread.run_sync(self._close)

    async def get_client(self, client_id: str) -> OAuthClientInformationFull | None:
        return await to_thread.run_sync(self._get_client, client_id)

    async def save_client(self, client: OAuthClientInformationFull) -> None:
        await to_thread.run_sync(self._save_client, client)

    async def save_token_pair(
        self,
        access: AccessToken,
        refresh: RefreshToken,
        family_id: str,
    ) -> None:
        await to_thread.run_sync(
            self._save_token_pair,
            access,
            refresh,
            family_id,
        )

    async def rotate_token_pair(
        self,
        old_refresh_token: str,
        access: AccessToken,
        refresh: RefreshToken,
    ) -> RefreshRotationOutcome:
        return await to_thread.run_sync(
            self._rotate_token_pair,
            old_refresh_token,
            access,
            refresh,
        )

    async def load_access_token(self, token: str) -> AccessToken | None:
        row = await to_thread.run_sync(self._load_token, token, "access")
        if row is None:
            return None
        return AccessToken(
            token=token,
            client_id=row["client_id"],
            scopes=json.loads(row["scopes_json"]),
            expires_at=row["expires_at"],
            resource=row["resource"],
            subject=row["subject"],
        )

    async def load_refresh_token(self, token: str) -> RefreshToken | None:
        row = await to_thread.run_sync(self._load_refresh_token, token)
        if row is None:
            return None
        return RefreshToken(
            token=token,
            client_id=row["client_id"],
            scopes=json.loads(row["scopes_json"]),
            expires_at=row["expires_at"],
            subject=row["subject"],
        )

    async def revoke_family(self, token: str) -> None:
        await to_thread.run_sync(self._revoke_family, token)

    async def count_token_families(self, client_id: str | None = None) -> int:
        return await to_thread.run_sync(self._count_token_families, client_id)

    def _initialize(self) -> None:
        with self._lock, self._connection:
            self._connection.executescript(
                """
                CREATE TABLE IF NOT EXISTS oauth_clients (
                    client_id TEXT PRIMARY KEY,
                    payload TEXT NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS oauth_tokens (
                    token_hash TEXT PRIMARY KEY,
                    token_kind TEXT NOT NULL,
                    family_id TEXT NOT NULL,
                    client_id TEXT NOT NULL,
                    scopes_json TEXT NOT NULL,
                    expires_at INTEGER,
                    resource TEXT,
                    subject TEXT
                );
                CREATE INDEX IF NOT EXISTS oauth_tokens_family
                    ON oauth_tokens(family_id);
                CREATE TABLE IF NOT EXISTS oauth_refresh_history (
                    token_hash TEXT PRIMARY KEY,
                    family_id TEXT NOT NULL,
                    expires_at INTEGER NOT NULL,
                    used_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS oauth_refresh_history_family
                    ON oauth_refresh_history(family_id);
                CREATE INDEX IF NOT EXISTS oauth_refresh_history_expiry
                    ON oauth_refresh_history(expires_at);
                """
            )
            columns = {
                row["name"]
                for row in self._connection.execute("PRAGMA table_info(oauth_clients)")
            }
            if "created_at" not in columns:
                self._connection.execute(
                    "ALTER TABLE oauth_clients "
                    "ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0"
                )

    def _close(self) -> None:
        with self._lock:
            self._connection.close()

    def _get_client(self, client_id: str) -> OAuthClientInformationFull | None:
        with self._lock:
            row = self._connection.execute(
                "SELECT payload FROM oauth_clients WHERE client_id = ?",
                (client_id,),
            ).fetchone()
        if row is None:
            return None
        return OAuthClientInformationFull.model_validate_json(row["payload"])

    def _save_client(self, client: OAuthClientInformationFull) -> None:
        if client.client_id is None:
            raise OAuthStoreConfigurationError("OAuth client_id is required.")
        payload = client.model_dump_json()
        with self._lock, self._connection:
            now = int(time.time())
            self._delete_expired_tokens_locked(now)
            exists = self._connection.execute(
                "SELECT 1 FROM oauth_clients WHERE client_id = ?",
                (client.client_id,),
            ).fetchone()
            if exists is None:
                self._make_client_room_locked(now)
            self._connection.execute(
                """
                INSERT INTO oauth_clients(client_id, payload, created_at)
                VALUES (?, ?, ?)
                ON CONFLICT(client_id) DO UPDATE SET payload = excluded.payload
                """,
                (client.client_id, payload, now),
            )

    def _save_token_pair(
        self,
        access: AccessToken,
        refresh: RefreshToken,
        family_id: str,
    ) -> None:
        with self._lock, self._connection:
            self._delete_expired_tokens_locked(int(time.time()))
            self._ensure_token_family_room_locked(access.client_id, family_id)
            self._insert_token(access, "access", family_id)
            self._insert_token(refresh, "refresh", family_id)

    def _rotate_token_pair(
        self,
        old_refresh_token: str,
        access: AccessToken,
        refresh: RefreshToken,
    ) -> RefreshRotationOutcome:
        if refresh.expires_at is None:
            raise OAuthStoreConfigurationError("Rotated refresh tokens must expire.")
        token_hash = _token_hash(old_refresh_token)
        with self._immediate_transaction():
            now = int(time.time())
            self._delete_expired_tokens_locked(now)
            live: sqlite3.Row | None = self._connection.execute(
                """SELECT family_id FROM oauth_tokens
                WHERE token_hash = ? AND token_kind = 'refresh'""",
                (token_hash,),
            ).fetchone()
            if live is None:
                replay: sqlite3.Row | None = self._connection.execute(
                    """SELECT family_id FROM oauth_refresh_history
                    WHERE token_hash = ?""",
                    (token_hash,),
                ).fetchone()
                if replay is None:
                    return RefreshRotationOutcome.MISSING
                self._delete_token_family_locked(str(replay["family_id"]))
                return RefreshRotationOutcome.REPLAYED
            family_id = str(live["family_id"])
            if self._refresh_history_at_capacity_locked(family_id):
                self._delete_token_family_locked(family_id)
                return RefreshRotationOutcome.HISTORY_EXHAUSTED
            family_expires_at = int(refresh.expires_at)
            _ = self._connection.execute(
                """UPDATE oauth_refresh_history SET expires_at = ?
                WHERE family_id = ?""",
                (family_expires_at, family_id),
            )
            _ = self._connection.execute(
                """INSERT INTO oauth_refresh_history(
                    token_hash, family_id, expires_at, used_at
                ) VALUES (?, ?, ?, ?)""",
                (token_hash, family_id, family_expires_at, now),
            )
            self._delete_token_family_locked(family_id)
            self._insert_token(access, "access", family_id)
            self._insert_token(refresh, "refresh", family_id)
            return RefreshRotationOutcome.ROTATED

    def _refresh_history_at_capacity_locked(self, family_id: str) -> bool:
        family_count = int(
            self._connection.execute(
                "SELECT COUNT(*) FROM oauth_refresh_history WHERE family_id = ?",
                (family_id,),
            ).fetchone()[0]
        )
        if family_count >= self._max_refresh_history_per_family:
            return True
        global_count = int(
            self._connection.execute(
                "SELECT COUNT(*) FROM oauth_refresh_history"
            ).fetchone()[0]
        )
        return global_count >= self._max_refresh_history_global

    def _delete_token_family_locked(self, family_id: str) -> None:
        _ = self._connection.execute(
            "DELETE FROM oauth_tokens WHERE family_id = ?",
            (family_id,),
        )

    @contextmanager
    def _immediate_transaction(self) -> Generator[None, None, None]:
        with self._lock, self._connection:
            _ = self._connection.execute("BEGIN IMMEDIATE")
            yield

    def _insert_token(
        self,
        token: AccessToken | RefreshToken,
        kind: str,
        family_id: str,
    ) -> None:
        resource = token.resource if isinstance(token, AccessToken) else None
        self._connection.execute(
            """
            INSERT INTO oauth_tokens(
                token_hash, token_kind, family_id, client_id, scopes_json,
                expires_at, resource, subject
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                _token_hash(token.token),
                kind,
                family_id,
                token.client_id,
                json.dumps(token.scopes),
                token.expires_at,
                resource,
                token.subject,
            ),
        )

    def _load_token(self, token: str, kind: str) -> sqlite3.Row | None:
        with self._lock:
            return self._connection.execute(
                "SELECT * FROM oauth_tokens WHERE token_hash = ? AND token_kind = ?",
                (_token_hash(token), kind),
            ).fetchone()

    def _load_refresh_token(self, token: str) -> sqlite3.Row | None:
        token_hash = _token_hash(token)
        with self._immediate_transaction():
            self._delete_expired_tokens_locked(int(time.time()))
            live: sqlite3.Row | None = self._connection.execute(
                """SELECT * FROM oauth_tokens
                WHERE token_hash = ? AND token_kind = 'refresh'""",
                (token_hash,),
            ).fetchone()
            if live is not None:
                return live
            replay: sqlite3.Row | None = self._connection.execute(
                """SELECT family_id FROM oauth_refresh_history
                WHERE token_hash = ?""",
                (token_hash,),
            ).fetchone()
            if replay is not None:
                self._delete_token_family_locked(str(replay["family_id"]))
            return None

    def _revoke_family(self, token: str) -> None:
        with self._immediate_transaction():
            self._delete_expired_tokens_locked(int(time.time()))
            token_hash = _token_hash(token)
            row: sqlite3.Row | None = self._connection.execute(
                "SELECT family_id FROM oauth_tokens WHERE token_hash = ?",
                (token_hash,),
            ).fetchone()
            if row is None:
                row = self._connection.execute(
                    """SELECT family_id FROM oauth_refresh_history
                    WHERE token_hash = ?""",
                    (token_hash,),
                ).fetchone()
            if row is not None:
                self._delete_token_family_locked(str(row["family_id"]))

    def _count_token_families(self, client_id: str | None) -> int:
        with self._lock, self._connection:
            self._delete_expired_tokens_locked(int(time.time()))
            return self._count_token_families_locked(client_id)

    def _ensure_token_family_room_locked(
        self,
        client_id: str,
        family_id: str,
    ) -> None:
        exists = self._connection.execute(
            "SELECT 1 FROM oauth_tokens WHERE family_id = ?",
            (family_id,),
        ).fetchone()
        if exists is not None:
            return
        if self._count_token_families_locked(client_id) >= (
            self._max_token_families_per_client
        ):
            raise OAuthTokenFamilyLimitError("OAuth token family client limit reached.")
        if self._count_token_families_locked(None) >= self._max_token_families:
            raise OAuthTokenFamilyLimitError("OAuth token family global limit reached.")

    def _count_token_families_locked(self, client_id: str | None) -> int:
        if client_id is None:
            row = self._connection.execute(
                "SELECT COUNT(DISTINCT family_id) FROM oauth_tokens"
            ).fetchone()
        else:
            row = self._connection.execute(
                "SELECT COUNT(DISTINCT family_id) FROM oauth_tokens "
                "WHERE client_id = ?",
                (client_id,),
            ).fetchone()
        return int(row[0])

    def _make_client_room_locked(self, now: int) -> None:
        count = int(
            self._connection.execute("SELECT COUNT(*) FROM oauth_clients").fetchone()[0]
        )
        required = count - self._max_clients + 1
        if required <= 0:
            return
        inactive = self._connection.execute(
            """
            SELECT client.client_id
            FROM oauth_clients AS client
            WHERE NOT EXISTS (
                SELECT 1
                FROM oauth_tokens AS token
                WHERE token.client_id = client.client_id
                  AND (token.expires_at IS NULL OR token.expires_at >= ?)
            )
            ORDER BY client.created_at ASC, client.rowid ASC
            LIMIT ?
            """,
            (now, required),
        ).fetchall()
        if len(inactive) < required:
            raise OAuthClientLimitError(
                "OAuth client storage is full with active registrations."
            )
        self._connection.executemany(
            "DELETE FROM oauth_clients WHERE client_id = ?",
            ((row["client_id"],) for row in inactive),
        )

    def _delete_expired_tokens_locked(self, now: int) -> None:
        self._connection.execute(
            "DELETE FROM oauth_tokens WHERE expires_at IS NOT NULL AND expires_at < ?",
            (now,),
        )
        self._connection.execute(
            "DELETE FROM oauth_refresh_history WHERE expires_at < ?",
            (now,),
        )


def _token_hash(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()
