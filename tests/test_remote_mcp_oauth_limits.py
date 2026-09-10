from __future__ import annotations

from pathlib import Path

import anyio
from fastapi.testclient import TestClient
from mcp.shared.auth import OAuthClientInformationFull

from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_store import OAuthStore
from tests.remote_mcp_oauth_support import oauth_settings


def test_pending_authorizations_have_a_hard_limit() -> None:
    settings = oauth_settings().model_copy(
        update={"oauth_pending_authorization_limit": 1}
    )
    app = create_app(settings)
    with TestClient(app, base_url="http://localhost") as client:
        registered = _register_client(client, "pending-limit")
        first = _start_authorization(client, registered, "first")
        second = _start_authorization(client, registered, "second")

    assert "/oauth/approve?" in first.headers["location"]
    assert "/oauth/approve?" not in second.headers.get("location", "")
    assert "temporarily_unavailable" in (
        second.headers.get("location", "") + second.text
    )


def test_client_store_prunes_the_oldest_inactive_registration(
    tmp_path: Path,
) -> None:
    async def scenario() -> None:
        store = OAuthStore(tmp_path / "oauth.sqlite3", max_clients=2)
        try:
            for name in ("client-a", "client-b", "client-c"):
                await store.save_client(_client(name))

            assert await store.get_client("client-a") is None
            assert await store.get_client("client-b") is not None
            assert await store.get_client("client-c") is not None
        finally:
            await store.close()

    anyio.run(scenario)


def test_nginx_limits_public_oauth_requests_per_ip() -> None:
    config = Path("remote_mcp_server/nginx-simdorei-mcp.conf").read_text(
        encoding="utf-8"
    )

    assert "limit_req_zone $binary_remote_addr" in config
    assert "limit_req zone=oauth_per_ip" in config
    assert "limit_req_status 429" in config


def _register_client(
    client: TestClient,
    name: str,
) -> dict[str, str]:
    response = client.post(
        "/register",
        json={
            "redirect_uris": ["https://chatgpt.com/connector/oauth/test"],
            "token_endpoint_auth_method": "client_secret_post",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_name": name,
        },
    )
    assert response.status_code == 201, response.text
    return {key: str(value) for key, value in response.json().items()}


def _start_authorization(
    client: TestClient,
    registered: dict[str, str],
    state: str,
):
    return client.get(
        "/authorize",
        params={
            "response_type": "code",
            "client_id": registered["client_id"],
            "redirect_uri": "https://chatgpt.com/connector/oauth/test",
            "state": state,
            "code_challenge": f"challenge-{state}-12345678901234567890",
            "code_challenge_method": "S256",
            "resource": "https://simdorei.duckdns.org/mcp",
        },
        follow_redirects=False,
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
