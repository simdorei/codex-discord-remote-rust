from __future__ import annotations

import hashlib
import sqlite3
import time
from pathlib import Path
from typing import cast
from unittest.mock import patch

import anyio
from fastapi.testclient import TestClient
from mcp.server.auth.provider import AccessToken, RefreshToken

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_store import (
    OAuthStore,
    RefreshRotationOutcome,
)
from tests.remote_mcp_oauth_support import authorize_grant, oauth_settings


def test_spent_refresh_replay_revokes_successor_after_restart(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        database_path = tmp_path / "oauth.sqlite3"
        first_access, first_refresh = _pair("first")
        next_access, next_refresh = _pair("next")
        attacker_access, attacker_refresh = _pair("attacker")

        store = OAuthStore(database_path)
        await store.save_token_pair(first_access, first_refresh, "family-a")
        rotated = await store.rotate_token_pair(
            first_refresh.token,
            next_access,
            next_refresh,
        )
        assert rotated is RefreshRotationOutcome.ROTATED
        await store.close()

        restarted = OAuthStore(database_path)
        try:
            replayed = await restarted.rotate_token_pair(
                first_refresh.token,
                attacker_access,
                attacker_refresh,
            )

            assert replayed is RefreshRotationOutcome.REPLAYED
            assert await restarted.load_access_token(next_access.token) is None
            assert await restarted.load_refresh_token(next_refresh.token) is None
        finally:
            await restarted.close()

    anyio.run(scenario)


def test_concurrent_refresh_reuse_revokes_the_winner_tokens(tmp_path: Path) -> None:
    async def scenario() -> None:
        database_path = tmp_path / "oauth.sqlite3"
        first_access, first_refresh = _pair("first")
        seed = OAuthStore(database_path)
        await seed.save_token_pair(first_access, first_refresh, "family-a")
        await seed.close()

        stores = (OAuthStore(database_path), OAuthStore(database_path))
        candidates = (_pair("candidate-a"), _pair("candidate-b"))
        outcomes: list[RefreshRotationOutcome] = []

        async def rotate(
            store: OAuthStore,
            pair: tuple[AccessToken, RefreshToken],
        ) -> None:
            outcomes.append(
                await store.rotate_token_pair(first_refresh.token, *pair)
            )

        try:
            async with anyio.create_task_group() as tasks:
                for store, pair in zip(stores, candidates, strict=True):
                    _ = tasks.start_soon(rotate, store, pair)
        finally:
            for store in stores:
                await store.close()

        assert outcomes.count(RefreshRotationOutcome.ROTATED) == 1
        assert outcomes.count(RefreshRotationOutcome.REPLAYED) == 1
        verifier = OAuthStore(database_path)
        try:
            for access, refresh in candidates:
                assert await verifier.load_access_token(access.token) is None
                assert await verifier.load_refresh_token(refresh.token) is None
        finally:
            await verifier.close()

    anyio.run(scenario)


def test_history_tracks_successor_expiry_not_spent_token_expiry(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        now = int(time.time())
        first_access, first_refresh = _pair(
            "first",
            now=now,
            refresh_expires_at=now + 10,
        )
        next_access, next_refresh = _pair(
            "next",
            now=now,
            refresh_expires_at=now + 3_600,
        )
        attacker_access, attacker_refresh = _pair("attacker", now=now)
        store = OAuthStore(tmp_path / "oauth.sqlite3")
        try:
            await store.save_token_pair(first_access, first_refresh, "family-a")
            outcome = await store.rotate_token_pair(
                first_refresh.token,
                next_access,
                next_refresh,
            )
            assert outcome is RefreshRotationOutcome.ROTATED

            with patch(
                "remote_mcp_server.simdorei_mcp.oauth_store.time.time",
                return_value=now + 11,
            ):
                replayed = await store.rotate_token_pair(
                    first_refresh.token,
                    attacker_access,
                    attacker_refresh,
                )

            assert replayed is RefreshRotationOutcome.REPLAYED
            assert await store.load_refresh_token(next_refresh.token) is None
        finally:
            await store.close()

    anyio.run(scenario)


def test_revoking_spent_refresh_token_revokes_active_successor(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        first_access, first_refresh = _pair("first")
        next_access, next_refresh = _pair("next")
        store = OAuthStore(tmp_path / "oauth.sqlite3")
        try:
            await store.save_token_pair(first_access, first_refresh, "family-a")
            outcome = await store.rotate_token_pair(
                first_refresh.token,
                next_access,
                next_refresh,
            )
            assert outcome is RefreshRotationOutcome.ROTATED

            await store.revoke_family(first_refresh.token)

            assert await store.load_access_token(next_access.token) is None
            assert await store.load_refresh_token(next_refresh.token) is None
        finally:
            await store.close()

    anyio.run(scenario)


def test_per_family_history_limit_revokes_instead_of_dropping_history(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        database_path = tmp_path / "oauth.sqlite3"
        first_access, first_refresh = _pair("first")
        next_access, next_refresh = _pair("next")
        final_access, final_refresh = _pair("final")
        store = OAuthStore(
            database_path,
            max_refresh_history_global=10,
            max_refresh_history_per_family=1,
        )
        try:
            await store.save_token_pair(first_access, first_refresh, "family-a")
            first_outcome = await store.rotate_token_pair(
                first_refresh.token,
                next_access,
                next_refresh,
            )
            exhausted = await store.rotate_token_pair(
                next_refresh.token,
                final_access,
                final_refresh,
            )

            assert first_outcome is RefreshRotationOutcome.ROTATED
            assert exhausted is RefreshRotationOutcome.HISTORY_EXHAUSTED
            assert await store.load_access_token(next_access.token) is None
            assert await store.load_refresh_token(next_refresh.token) is None
        finally:
            await store.close()

        assert _history_count(database_path) == 1

    anyio.run(scenario)


def test_global_history_limit_revokes_only_the_rotating_family(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        database_path = tmp_path / "oauth.sqlite3"
        store = OAuthStore(
            database_path,
            max_refresh_history_global=1,
            max_refresh_history_per_family=1,
        )
        a_first = _pair("a-first", client_id="client-a")
        a_next = _pair("a-next", client_id="client-a")
        b_first = _pair("b-first", client_id="client-b")
        b_next = _pair("b-next", client_id="client-b")
        try:
            await store.save_token_pair(*a_first, "family-a")
            await store.save_token_pair(*b_first, "family-b")
            rotated = await store.rotate_token_pair(a_first[1].token, *a_next)
            exhausted = await store.rotate_token_pair(b_first[1].token, *b_next)

            assert rotated is RefreshRotationOutcome.ROTATED
            assert exhausted is RefreshRotationOutcome.HISTORY_EXHAUSTED
            assert await store.load_refresh_token(a_next[1].token) is not None
            assert await store.load_refresh_token(b_first[1].token) is None
        finally:
            await store.close()

        assert _history_count(database_path) == 1

    anyio.run(scenario)


def test_existing_oauth_database_adds_refresh_history_schema(tmp_path: Path) -> None:
    database_path = tmp_path / "oauth.sqlite3"
    legacy_access, legacy_refresh = _pair("legacy")
    connection = sqlite3.connect(database_path)
    try:
        _ = connection.executescript(
            """
            CREATE TABLE oauth_clients (
                client_id TEXT PRIMARY KEY,
                payload TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE oauth_tokens (
                token_hash TEXT PRIMARY KEY,
                token_kind TEXT NOT NULL,
                family_id TEXT NOT NULL,
                client_id TEXT NOT NULL,
                scopes_json TEXT NOT NULL,
                expires_at INTEGER,
                resource TEXT,
                subject TEXT
            );
            """
        )
        _ = connection.executemany(
            """INSERT INTO oauth_tokens(
                token_hash, token_kind, family_id, client_id, scopes_json,
                expires_at, resource, subject
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                (
                    _token_hash(legacy_access.token),
                    "access",
                    "family-legacy",
                    legacy_access.client_id,
                    '["mcp:read"]',
                    legacy_access.expires_at,
                    legacy_access.resource,
                    legacy_access.subject,
                ),
                (
                    _token_hash(legacy_refresh.token),
                    "refresh",
                    "family-legacy",
                    legacy_refresh.client_id,
                    '["mcp:read"]',
                    legacy_refresh.expires_at,
                    None,
                    legacy_refresh.subject,
                ),
            ),
        )
        connection.commit()
    finally:
        connection.close()

    async def initialize() -> None:
        store = OAuthStore(database_path)
        try:
            assert await store.load_access_token(legacy_access.token) is not None
            assert await store.load_refresh_token(legacy_refresh.token) is not None
        finally:
            await store.close()

    anyio.run(initialize)
    check = sqlite3.connect(database_path)
    try:
        row = cast(
            "tuple[str] | None",
            check.execute(
                """SELECT name FROM sqlite_master
                WHERE type = 'table' AND name = 'oauth_refresh_history'"""
            ).fetchone(),
        )
    finally:
        check.close()

    assert row == ("oauth_refresh_history",)


def test_http_refresh_replay_revokes_successor_without_replay_detail(
    tmp_path: Path,
) -> None:
    app = create_app(oauth_settings(tmp_path / "oauth.sqlite3"))
    with TestClient(app, base_url="http://localhost") as client:
        grant = authorize_grant(client)
        refresh_data = {
            "grant_type": "refresh_token",
            "refresh_token": grant.refresh_token,
            "client_id": grant.client_id,
            "client_secret": grant.client_secret,
            "resource": "https://simdorei.duckdns.org/mcp",
        }
        refreshed = client.post("/token", data=refresh_data)
        assert refreshed.status_code == 200, refreshed.text
        payload = cast("dict[str, object]", refreshed.json())
        successor_access = str(payload["access_token"])

        replayed = client.post("/token", data=refresh_data)
        protected = client.post(
            "/mcp",
            headers={
                "Accept": "application/json, text/event-stream",
                "Content-Type": "application/json",
                "Authorization": f"Bearer {successor_access}",
            },
            json={"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
        )

    assert replayed.status_code == 400
    assert replayed.json()["error"] == "invalid_grant"
    assert "replay" not in replayed.text.lower()
    assert protected.status_code == 401


def _history_count(database_path: Path) -> int:
    connection = sqlite3.connect(database_path)
    try:
        row = cast(
            "tuple[int]",
            connection.execute(
                "SELECT COUNT(*) FROM oauth_refresh_history"
            ).fetchone(),
        )
        return row[0]
    finally:
        connection.close()


def _token_hash(token: str) -> str:
    return hashlib.sha256(token.encode("utf-8")).hexdigest()


def _pair(
    label: str,
    *,
    client_id: str = "client-a",
    now: int | None = None,
    refresh_expires_at: int | None = None,
) -> tuple[AccessToken, RefreshToken]:
    issued_at = int(time.time()) if now is None else now
    return (
        AccessToken(
            token=f"access-{label}",
            client_id=client_id,
            scopes=["mcp:read"],
            expires_at=issued_at + 3_600,
            resource="https://example.test/mcp",
            subject="owner",
        ),
        RefreshToken(
            token=f"refresh-{label}",
            client_id=client_id,
            scopes=["mcp:read"],
            expires_at=(
                issued_at + 86_400
                if refresh_expires_at is None
                else refresh_expires_at
            ),
            subject="owner",
        ),
    )
