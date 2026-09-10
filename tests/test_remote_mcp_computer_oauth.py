from __future__ import annotations

from dataclasses import dataclass
from urllib.parse import parse_qs, urlparse

import pytest
from fastapi.testclient import TestClient
from mcp.server.auth.provider import AccessToken
from mcp.server.fastmcp.exceptions import ToolError

from remote_mcp_server.simdorei_mcp import tool_context
from remote_mcp_server.simdorei_mcp.app import create_app
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_REQUIRED_SCOPES,
    COMPUTER_CONTROL_SCOPE,
    COMPUTER_OBSERVE_REQUIRED_SCOPES,
    COMPUTER_OBSERVE_SCOPE,
    READ_SCOPE,
    WRITE_SCOPE,
)
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from remote_mcp_server.simdorei_mcp.tool_context import (
    OpenAiToolSession,
    ToolIdentity,
    tool_request_id,
)
from tests.remote_mcp_oauth_support import authorize, authorize_grant, oauth_settings

MCP_HEADERS = {
    "Accept": "application/json, text/event-stream",
    "Content-Type": "application/json",
}


@dataclass(frozen=True, slots=True)
class _IdentityRequestContext:
    meta: OpenAiToolSession


@dataclass(frozen=True, slots=True)
class _IdentityContext:
    request_context: _IdentityRequestContext
    request_id: str = "same-upstream-request"


def _identity_context(session: str) -> _IdentityContext:
    return _IdentityContext(
        request_context=_IdentityRequestContext(
            meta=OpenAiToolSession.model_validate({"openai/session": session})
        )
    )


def test_existing_file_only_oauth_grant_cannot_gain_computer_control() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client, (READ_SCOPE, WRITE_SCOPE))
        response = client.post(
            "/mcp",
            headers={
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            },
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "screenshot_computer_window",
                    "arguments": {"window_id": 42},
                    "_meta": {"openai/session": "old-file-only-session"},
                },
            },
        )

    assert response.status_code == 200, response.text
    result = response.json()["result"]
    assert result["isError"] is True
    assert "computer:observe" in result["content"][0]["text"]


def test_file_only_refresh_token_cannot_add_computer_scopes() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        grant = authorize_grant(client, (READ_SCOPE, WRITE_SCOPE))
        response = client.post(
            "/token",
            data={
                "grant_type": "refresh_token",
                "refresh_token": grant.refresh_token,
                "client_id": grant.client_id,
                "client_secret": grant.client_secret,
                "scope": "files:read files:write computer:observe computer:control",
                "resource": "https://simdorei.duckdns.org/mcp",
            },
        )

    assert response.status_code == 400
    assert response.json()["error"] == "invalid_scope"


def test_existing_oauth_client_can_request_new_server_scopes() -> None:
    app = create_app(oauth_settings())
    redirect_uri = "https://chatgpt.com/connector/oauth/test"
    with TestClient(app, base_url="http://localhost") as client:
        registered = client.post(
            "/register",
            json={
                "redirect_uris": [redirect_uri],
                "token_endpoint_auth_method": "client_secret_post",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "scope": f"{READ_SCOPE} {WRITE_SCOPE}",
                "client_name": "Existing ChatGPT client",
            },
        )
        assert registered.status_code == 201, registered.text

        authorization = client.get(
            "/authorize",
            params={
                "response_type": "code",
                "client_id": registered.json()["client_id"],
                "redirect_uri": redirect_uri,
                "state": "existing-client-state",
                "code_challenge": "a" * 43,
                "code_challenge_method": "S256",
                "scope": (
                    f"{READ_SCOPE} {WRITE_SCOPE} "
                    f"{COMPUTER_OBSERVE_SCOPE} {COMPUTER_CONTROL_SCOPE}"
                ),
                "resource": "https://simdorei.duckdns.org/mcp",
            },
            follow_redirects=False,
        )

    assert authorization.status_code == 302, authorization.text
    assert urlparse(authorization.headers["location"]).path == "/oauth/approve"


def test_oauth_approval_form_preserves_public_path_prefix() -> None:
    settings = GatewaySettings.model_validate(
        {
            **oauth_settings().model_dump(),
            "public_base_url": "https://simdorei.duckdns.org/v12",
        }
    )
    app = create_app(settings)
    redirect_uri = "https://chatgpt.com/connector/oauth/test"
    with TestClient(app, base_url="http://localhost") as client:
        registered = client.post(
            "/register",
            json={
                "redirect_uris": [redirect_uri],
                "token_endpoint_auth_method": "client_secret_post",
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "scope": READ_SCOPE,
                "client_name": "Prefixed ChatGPT client",
            },
        )
        assert registered.status_code == 201, registered.text
        authorization = client.get(
            "/authorize",
            params={
                "response_type": "code",
                "client_id": registered.json()["client_id"],
                "redirect_uri": redirect_uri,
                "state": "prefixed-state",
                "code_challenge": "a" * 43,
                "code_challenge_method": "S256",
                "scope": READ_SCOPE,
                "resource": "https://simdorei.duckdns.org/v12/mcp",
            },
            follow_redirects=False,
        )
        assert authorization.status_code == 302, authorization.text
        approval_url = urlparse(authorization.headers["location"])
        request_id = parse_qs(approval_url.query)["request_id"][0]
        approval_page = client.get(
            "/oauth/approve",
            params={"request_id": request_id},
        )

    assert approval_url.path == "/v12/oauth/approve"
    assert approval_page.status_code == 200
    assert 'action="/v12/oauth/approve"' in approval_page.text


def test_oauth_request_without_scope_grants_full_local_computer_access() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        grant = authorize_grant(client, None)
        response = client.post(
            "/mcp",
            headers={
                **MCP_HEADERS,
                "Authorization": f"Bearer {grant.access_token}",
            },
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "screenshot_computer_window",
                    "arguments": {"window_id": 42},
                    "_meta": {"openai/session": "no-scope-session"},
                },
            },
        )

    result = response.json()["result"]
    assert result["isError"] is True
    assert "computer:observe" not in result["content"][0]["text"]
    assert "computer:observe" in grant.approval_page
    assert "computer:control" in grant.approval_page


def test_oauth_approval_discloses_requested_computer_capabilities() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        grant = authorize_grant(
            client,
            (COMPUTER_OBSERVE_SCOPE, COMPUTER_CONTROL_SCOPE),
        )

    assert "computer:observe" in grant.approval_page
    assert "computer:control" in grant.approval_page
    assert "screenshots" in grant.approval_page.casefold()
    assert "keyboard" in grant.approval_page.casefold()


def test_computer_control_requires_observe_scope_too() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client, (COMPUTER_CONTROL_SCOPE,))
        response = client.post(
            "/mcp",
            headers={
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            },
            json={
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "launch_computer_app",
                    "arguments": {"app": "notepad"},
                    "_meta": {"openai/session": "control-only-session"},
                },
            },
        )

    assert response.status_code == 403


def test_runtime_scope_check_requires_every_advertised_control_scope(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    context = _identity_context("control-only-session")
    access_token = AccessToken(
        token="control-only-token",
        client_id="client-a",
        scopes=[COMPUTER_CONTROL_SCOPE],
        expires_at=9_999_999_999,
        resource="https://simdorei.duckdns.org/mcp",
        subject="owner",
    )
    monkeypatch.setattr(tool_context, "get_access_token", lambda: access_token)

    with pytest.raises(ToolError, match="files:read"):
        tool_context.tool_identity(context, COMPUTER_CONTROL_REQUIRED_SCOPES)


def test_oauth_client_is_part_of_the_tool_identity(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    context = _identity_context("shared-session")
    tokens = [
        AccessToken(
            token=f"token-{client_id}",
            client_id=client_id,
            scopes=list(COMPUTER_OBSERVE_REQUIRED_SCOPES),
            expires_at=9_999_999_999,
            resource="https://simdorei.duckdns.org/mcp",
            subject="owner",
        )
        for client_id in ("client-a", "client-b")
    ]
    monkeypatch.setattr(tool_context, "get_access_token", lambda: tokens.pop(0))

    first = tool_context.tool_identity(context, COMPUTER_OBSERVE_REQUIRED_SCOPES)
    second = tool_context.tool_identity(context, COMPUTER_OBSERVE_REQUIRED_SCOPES)

    assert first.session == second.session == "shared-session"
    assert first.subject != second.subject


def test_repeated_upstream_request_id_gets_a_fresh_local_invocation_id() -> None:
    context = _identity_context("shared-session")
    identity = ToolIdentity(session="shared-session", subject="subject-a")

    first = tool_request_id(context, identity)
    second = tool_request_id(context, identity)

    assert first != second


def test_computer_tools_publish_dedicated_scopes_truthful_risk_and_bounds() -> None:
    app = create_app(oauth_settings())
    with TestClient(app, base_url="http://localhost") as client:
        access_token = authorize(client)
        response = client.post(
            "/mcp",
            headers={
                **MCP_HEADERS,
                "Authorization": f"Bearer {access_token}",
            },
            json={"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}},
        )

    assert response.status_code == 200, response.text
    tools = {tool["name"]: tool for tool in response.json()["result"]["tools"]}
    screenshot = tools["screenshot_computer_window"]
    click = tools["click_computer_window"]
    typing = tools["type_computer_text"]
    assert screenshot["annotations"] == {
        "readOnlyHint": True,
        "destructiveHint": False,
        "openWorldHint": True,
    }
    assert screenshot["_meta"]["securitySchemes"][0]["scopes"] == [
        "files:read",
        "computer:observe",
    ]
    assert click["annotations"]["destructiveHint"] is True
    assert click["annotations"]["openWorldHint"] is True
    assert click["_meta"]["securitySchemes"][0]["scopes"] == [
        "files:read",
        "computer:observe",
        "computer:control",
    ]
    assert click["inputSchema"]["properties"]["window_id"]["exclusiveMinimum"] == 0
    assert click["inputSchema"]["properties"]["observation_id"]["minLength"] == 8
    assert click["inputSchema"]["properties"]["x"]["maximum"] == 100_000
    assert click["inputSchema"]["properties"]["click_count"]["maximum"] == 3
    assert typing["inputSchema"]["properties"]["text"]["maxLength"] == 4_096
