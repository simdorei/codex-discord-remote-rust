from __future__ import annotations

from pathlib import Path
from typing import override
from urllib.parse import parse_qs, urlparse

import anyio
from mcp.server.auth.provider import (
    AccessToken,
    AuthorizationCode,
    AuthorizationParams,
    RefreshToken,
)
from mcp.shared.auth import OAuthClientInformationFull
from pydantic import AnyUrl, SecretStr

from remote_mcp_server.simdorei_mcp.oauth_provider import SingleUserOAuthProvider
from remote_mcp_server.simdorei_mcp.oauth_store import OAuthStore


def test_cancelled_code_exchange_restores_code_and_propagates_cancellation() -> None:
    async def scenario() -> None:
        entered = anyio.Event()
        provider = _provider(_BlockingOAuthStore(entered))
        client = _client("client-a")
        cancel_scope = anyio.CancelScope()
        cancelled_exc_class = anyio.get_cancelled_exc_class()
        caught: list[type[BaseException]] = []
        try:
            code = await _issue_code(provider, client, "cancelled")

            async def exchange() -> None:
                with cancel_scope:
                    try:
                        _ = await provider.exchange_authorization_code(client, code)
                    except cancelled_exc_class as exc:
                        caught.append(type(exc))
                        raise

            async with anyio.create_task_group() as tasks:
                _ = tasks.start_soon(exchange)
                await entered.wait()
                cancel_scope.cancel()

            restored = await provider.load_authorization_code(client, code.code)
            assert caught == [cancelled_exc_class]
            assert restored == code
        finally:
            await provider.close()

    anyio.run(scenario)


class _BlockingOAuthStore(OAuthStore):
    _entered: anyio.Event

    def __init__(self, entered: anyio.Event) -> None:
        super().__init__(Path(":memory:"))
        self._entered = entered

    @override
    async def save_token_pair(
        self,
        access: AccessToken,
        refresh: RefreshToken,
        family_id: str,
    ) -> None:
        del access, refresh, family_id
        self._entered.set()
        await anyio.sleep_forever()


def _provider(store: OAuthStore) -> SingleUserOAuthProvider:
    return SingleUserOAuthProvider(
        store=store,
        owner_token=SecretStr("owner-secret-12345678901234567890"),
        public_base_url="https://simdorei.duckdns.org",
        resource_url="https://simdorei.duckdns.org/mcp",
        access_token_seconds=3_600,
        refresh_token_seconds=86_400,
        pending_authorization_limit=10,
        authorization_code_limit=10,
        authorization_code_per_client_limit=10,
    )


def _client(client_id: str) -> OAuthClientInformationFull:
    return OAuthClientInformationFull.model_validate(
        {
            "redirect_uris": ["https://chatgpt.com/connector/oauth/test"],
            "token_endpoint_auth_method": "client_secret_post",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_name": client_id,
            "client_id": client_id,
            "client_secret": f"secret-{client_id}",
        }
    )


async def _issue_code(
    provider: SingleUserOAuthProvider,
    client: OAuthClientInformationFull,
    state: str,
) -> AuthorizationCode:
    params = AuthorizationParams(
        state=state,
        scopes=["project:read"],
        code_challenge=f"challenge-{state}-12345678901234567890",
        redirect_uri=AnyUrl("https://chatgpt.com/connector/oauth/test"),
        redirect_uri_provided_explicitly=True,
        resource="https://simdorei.duckdns.org/mcp",
    )
    approval_url = await provider.authorize(client, params)
    request_id = parse_qs(urlparse(approval_url).query)["request_id"][0]
    callback = await provider.approve(
        request_id,
        "owner-secret-12345678901234567890",
    )
    code_value = parse_qs(urlparse(callback).query)["code"][0]
    code = await provider.load_authorization_code(client, code_value)
    assert code is not None
    return code
