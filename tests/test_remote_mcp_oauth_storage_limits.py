from __future__ import annotations

import base64
import hashlib
from pathlib import Path
from typing import override
from urllib.parse import parse_qs, urlparse

import anyio
import pytest
from fastapi.testclient import TestClient
from mcp.server.auth.provider import (
    AccessToken,
    AuthorizationCode,
    AuthorizationParams,
    RefreshToken,
    TokenError,
)
from mcp.shared.auth import OAuthClientInformationFull
from pydantic import AnyUrl, SecretStr

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_provider import (
    ApprovalCapacityError,
    SingleUserOAuthProvider,
    TokenCapacityError,
)
from remote_mcp_server.simdorei_mcp.oauth_store import (
    OAuthStore,
    OAuthTokenFamilyLimitError,
    RefreshRotationOutcome,
)
from tests.remote_mcp_oauth_support import authorize_grant, oauth_settings
from tests.test_remote_mcp_oauth_limits import (
    _client,
    _register_client,
    _start_authorization,
)


@pytest.mark.parametrize(
    ("global_limit", "per_client_limit", "same_client"),
    ((1, 1, False), (2, 1, True)),
    ids=("global", "per-client"),
)
def test_authorization_code_limit_returns_503_and_preserves_pending(
    global_limit: int,
    per_client_limit: int,
    same_client: bool,
) -> None:
    settings = oauth_settings().model_copy(
        update={
            "oauth_authorization_code_global_limit": global_limit,
            "oauth_authorization_code_per_client_limit": per_client_limit,
        }
    )
    app = create_app(settings)
    with TestClient(app, base_url="http://localhost") as client:
        first_client = _register_client(client, "code-limit-first")
        second_client = (
            first_client
            if same_client
            else _register_client(client, "code-limit-second")
        )
        first = _start_authorization(client, first_client, "first")
        second = _start_authorization(client, second_client, "second")
        first_id = _request_id(first.headers["location"])
        second_id = _request_id(second.headers["location"])

        first_approval = _approve(client, first_id)
        second_approval = _approve(client, second_id)
        preserved = client.get("/oauth/approve", params={"request_id": second_id})

    assert first_approval.status_code == 302
    assert second_approval.status_code == 503
    assert "temporarily_unavailable" in second_approval.text
    assert preserved.status_code == 200


def test_token_family_limits_are_global_and_per_client(tmp_path: Path) -> None:
    async def scenario() -> None:
        store = OAuthStore(
            tmp_path / "oauth.sqlite3",
            max_token_families=2,
            max_token_families_per_client=1,
        )
        try:
            await store.save_token_pair(*_pair("a-1", "client-a"), "family-a")
            with pytest.raises(OAuthTokenFamilyLimitError, match="client"):
                await store.save_token_pair(*_pair("a-2", "client-a"), "family-a2")
            await store.save_token_pair(*_pair("b-1", "client-b"), "family-b")
            with pytest.raises(OAuthTokenFamilyLimitError, match="global"):
                await store.save_token_pair(*_pair("c-1", "client-c"), "family-c")

            assert await store.count_token_families() == 2
            assert await store.count_token_families("client-a") == 1
        finally:
            await store.close()

    anyio.run(scenario)


def test_token_family_limit_returns_explicit_oauth_error() -> None:
    settings = oauth_settings().model_copy(
        update={
            "oauth_token_family_global_limit": 1,
            "oauth_token_family_per_client_limit": 1,
        }
    )
    app = create_app(settings)
    with TestClient(app, base_url="http://localhost") as client:
        authorize_grant(client)
        registered = _register_client(client, "token-limit")
        verifier = "oauth-capacity-verifier-123456789012345678901234567890"
        challenge = (
            base64.urlsafe_b64encode(hashlib.sha256(verifier.encode("utf-8")).digest())
            .decode("ascii")
            .rstrip("=")
        )
        authorization = client.get(
            "/authorize",
            params={
                "response_type": "code",
                "client_id": registered["client_id"],
                "redirect_uri": "https://chatgpt.com/connector/oauth/test",
                "state": "token-limit",
                "code_challenge": challenge,
                "code_challenge_method": "S256",
                "resource": "https://simdorei.duckdns.org/mcp",
            },
            follow_redirects=False,
        )
        request_id = _request_id(authorization.headers["location"])
        approved = _approve(client, request_id)
        code = parse_qs(urlparse(approved.headers["location"]).query)["code"][0]
        token = client.post(
            "/token",
            data={
                "grant_type": "authorization_code",
                "code": code,
                "client_id": registered["client_id"],
                "client_secret": registered["client_secret"],
                "redirect_uri": "https://chatgpt.com/connector/oauth/test",
                "code_verifier": verifier,
                "resource": "https://simdorei.duckdns.org/mcp",
            },
        )

    assert token.status_code == 503
    assert token.json()["error"] == "temporarily_unavailable"


def test_refresh_rotation_replaces_one_family_at_the_limit(tmp_path: Path) -> None:
    async def scenario() -> None:
        store = OAuthStore(
            tmp_path / "oauth.sqlite3",
            max_token_families=1,
            max_token_families_per_client=1,
        )
        try:
            first_access, first_refresh = _pair("first", "client-a")
            await store.save_token_pair(first_access, first_refresh, "family-first")
            next_access, next_refresh = _pair("next", "client-a")

            rotated = await store.rotate_token_pair(
                first_refresh.token,
                next_access,
                next_refresh,
            )

            assert rotated is RefreshRotationOutcome.ROTATED
            assert await store.count_token_families() == 1
            assert await store.load_refresh_token(next_refresh.token) is not None
            assert await store.load_refresh_token(first_refresh.token) is None
            assert await store.load_refresh_token(next_refresh.token) is None
        finally:
            await store.close()

    anyio.run(scenario)


def test_failed_code_exchange_blocks_concurrent_use_then_restores_code() -> None:
    async def scenario() -> None:
        entered = anyio.Event()
        release = anyio.Event()
        store = _BlockingFailingOAuthStore(entered, release)
        provider = _provider(store, authorization_code_limit=1)
        client = _client("client-a")
        try:
            code = await _issue_code(provider, client, "first")

            first_errors: list[str] = []

            async def first_exchange() -> None:
                with pytest.raises(TokenCapacityError):
                    await provider.exchange_authorization_code(client, code)
                first_errors.append("temporarily_unavailable")

            async with anyio.create_task_group() as tasks:
                tasks.start_soon(first_exchange)
                await entered.wait()
                with pytest.raises(TokenError) as caught:
                    await provider.exchange_authorization_code(client, code)
                assert caught.value.error == "invalid_grant"
                second_approval = await provider.authorize(
                    client,
                    _authorization_params("second"),
                )
                second_request_id = _request_id(second_approval)
                with pytest.raises(ApprovalCapacityError):
                    await provider.approve(
                        second_request_id,
                        "owner-secret-12345678901234567890",
                    )
                assert await provider.pending_scopes(second_request_id) is not None
                release.set()

            assert first_errors == ["temporarily_unavailable"]
            assert await provider.load_authorization_code(client, code.code) is not None
        finally:
            await provider.close()

    anyio.run(scenario)


def test_unexpected_code_exchange_failure_restores_code() -> None:
    async def scenario() -> None:
        provider = _provider(_UnexpectedFailingOAuthStore())
        client = _client("client-a")
        try:
            code = await _issue_code(provider, client, "unexpected")
            with pytest.raises(RuntimeError, match="unexpected storage failure"):
                await provider.exchange_authorization_code(client, code)
            assert await provider.load_authorization_code(client, code.code) is not None
        finally:
            await provider.close()

    anyio.run(scenario)


def _pair(label: str, client_id: str) -> tuple[AccessToken, RefreshToken]:
    expires_at = 9_999_999_999
    return (
        AccessToken(
            token=f"access-{label}",
            client_id=client_id,
            scopes=["project:read"],
            expires_at=expires_at,
            resource="https://simdorei.duckdns.org/mcp",
            subject="owner",
        ),
        RefreshToken(
            token=f"refresh-{label}",
            client_id=client_id,
            scopes=["project:read"],
            expires_at=expires_at,
            subject="owner",
        ),
    )


def _request_id(location: str) -> str:
    return parse_qs(urlparse(location).query)["request_id"][0]


def _approve(client: TestClient, request_id: str):
    return client.post(
        "/oauth/approve",
        data={
            "request_id": request_id,
            "owner_token": "owner-secret-12345678901234567890",
        },
        follow_redirects=False,
    )


def _provider(
    store: OAuthStore,
    *,
    authorization_code_limit: int = 10,
) -> SingleUserOAuthProvider:
    return SingleUserOAuthProvider(
        store=store,
        owner_token=SecretStr("owner-secret-12345678901234567890"),
        public_base_url="https://simdorei.duckdns.org",
        resource_url="https://simdorei.duckdns.org/mcp",
        access_token_seconds=3_600,
        refresh_token_seconds=86_400,
        pending_authorization_limit=10,
        authorization_code_limit=authorization_code_limit,
        authorization_code_per_client_limit=authorization_code_limit,
    )


async def _issue_code(
    provider: SingleUserOAuthProvider,
    client: OAuthClientInformationFull,
    state: str,
) -> AuthorizationCode:
    approval_url = await provider.authorize(client, _authorization_params(state))
    callback = await provider.approve(
        _request_id(approval_url),
        "owner-secret-12345678901234567890",
    )
    code_value = parse_qs(urlparse(callback).query)["code"][0]
    code = await provider.load_authorization_code(client, code_value)
    assert code is not None
    return code


def _authorization_params(state: str) -> AuthorizationParams:
    return AuthorizationParams(
        state=state,
        scopes=["project:read"],
        code_challenge=f"challenge-{state}-12345678901234567890",
        redirect_uri=AnyUrl("https://chatgpt.com/connector/oauth/test"),
        redirect_uri_provided_explicitly=True,
        resource="https://simdorei.duckdns.org/mcp",
    )


class _BlockingFailingOAuthStore(OAuthStore):
    def __init__(self, entered: anyio.Event, release: anyio.Event) -> None:
        super().__init__(Path(":memory:"))
        self._entered = entered
        self._release = release

    @override
    async def save_token_pair(
        self,
        access: AccessToken,
        refresh: RefreshToken,
        family_id: str,
    ) -> None:
        del access, refresh, family_id
        self._entered.set()
        await self._release.wait()
        raise OAuthTokenFamilyLimitError("test capacity reached")


class _UnexpectedFailingOAuthStore(OAuthStore):
    def __init__(self) -> None:
        super().__init__(Path(":memory:"))

    @override
    async def save_token_pair(
        self,
        access: AccessToken,
        refresh: RefreshToken,
        family_id: str,
    ) -> None:
        del access, refresh, family_id
        raise RuntimeError("unexpected storage failure")
