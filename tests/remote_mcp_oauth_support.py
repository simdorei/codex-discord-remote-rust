from __future__ import annotations

import base64
import hashlib
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import parse_qs, urlparse

from fastapi.testclient import TestClient

from remote_mcp_server.simdorei_mcp.oauth_scopes import OAUTH_SCOPES
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from simdorei_mcp_common.connector_contract import PRODUCTION_CONNECTOR_RESOURCE

DEFAULT_OAUTH_SCOPES = tuple(OAUTH_SCOPES)
BRIDGE_DEVICE_TOKEN = "a" * 40


@dataclass(frozen=True, slots=True)
class OAuthGrant:
    access_token: str
    refresh_token: str
    client_id: str
    client_secret: str
    approval_page: str


def oauth_settings(oauth_database_path: Path | None = None) -> GatewaySettings:
    return GatewaySettings.model_validate(
        {
            "device_credentials": {
                "version": 1,
                "devices": [
                    {"device_id": "device-a", "token": BRIDGE_DEVICE_TOKEN},
                ],
            },
            "public_base_url": "https://simdorei.duckdns.org",
            "owner_token": "owner-secret-12345678901234567890",
            "oauth_database_path": oauth_database_path or Path(":memory:"),
        }
    )


def authorize(
    client: TestClient,
    scopes: tuple[str, ...] = DEFAULT_OAUTH_SCOPES,
    *,
    resource: str = PRODUCTION_CONNECTOR_RESOURCE,
) -> str:
    return authorize_grant(client, scopes, resource=resource).access_token


def authorize_grant(
    client: TestClient,
    scopes: tuple[str, ...] | None = DEFAULT_OAUTH_SCOPES,
    *,
    resource: str = PRODUCTION_CONNECTOR_RESOURCE,
) -> OAuthGrant:
    redirect_uri = "https://chatgpt.com/connector/oauth/test"
    scope_text = " ".join(scopes) if scopes is not None else None
    registration: dict[str, str | list[str]] = {
        "redirect_uris": [redirect_uri],
        "token_endpoint_auth_method": "client_secret_post",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "client_name": "ChatGPT integration test",
    }
    if scope_text is not None:
        registration["scope"] = scope_text
    registered = client.post(
        "/register",
        json=registration,
    )
    assert registered.status_code == 201, registered.text
    client_info = registered.json()
    verifier = "oauth-pkce-verifier-123456789012345678901234567890"
    challenge = base64.urlsafe_b64encode(
        hashlib.sha256(verifier.encode("utf-8")).digest()
    ).decode("ascii").rstrip("=")
    authorization_params: dict[str, str] = {
        "response_type": "code",
        "client_id": client_info["client_id"],
        "redirect_uri": redirect_uri,
        "state": "integration-state",
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "resource": resource,
    }
    if scope_text is not None:
        authorization_params["scope"] = scope_text
    authorization = client.get(
        "/authorize",
        params=authorization_params,
        follow_redirects=False,
    )
    assert authorization.status_code == 302, authorization.text
    request_id = parse_qs(urlparse(authorization.headers["location"]).query)[
        "request_id"
    ][0]
    approval_page = client.get("/oauth/approve", params={"request_id": request_id})
    assert approval_page.status_code == 200
    approved = client.post(
        "/oauth/approve",
        data={
            "request_id": request_id,
            "owner_token": "owner-secret-12345678901234567890",
        },
        follow_redirects=False,
    )
    assert approved.status_code == 302, approved.text
    callback_query = parse_qs(urlparse(approved.headers["location"]).query)
    assert callback_query["state"] == ["integration-state"]
    token = client.post(
        "/token",
        data={
            "grant_type": "authorization_code",
            "code": callback_query["code"][0],
            "client_id": client_info["client_id"],
            "client_secret": client_info["client_secret"],
            "redirect_uri": redirect_uri,
            "code_verifier": verifier,
            "resource": resource,
        },
    )
    assert token.status_code == 200, token.text
    token_json = token.json()
    return OAuthGrant(
        access_token=str(token_json["access_token"]),
        refresh_token=str(token_json["refresh_token"]),
        client_id=str(client_info["client_id"]),
        client_secret=str(client_info["client_secret"]),
        approval_page=approval_page.text,
    )
