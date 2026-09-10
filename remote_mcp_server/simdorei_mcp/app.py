from __future__ import annotations

from builtins import BaseExceptionGroup
from collections.abc import AsyncGenerator, Awaitable, Callable
from contextlib import AbstractAsyncContextManager, asynccontextmanager
from typing import ClassVar

from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from mcp.server.auth.settings import (
    AuthSettings,
    ClientRegistrationOptions,
    RevocationOptions,
)
from mcp.server.fastmcp import FastMCP
from mcp.server.transport_security import TransportSecuritySettings
from pydantic import AnyHttpUrl, BaseModel, ConfigDict

from remote_mcp_server.simdorei_mcp.bridge_router import create_bridge_router
from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.capability_inventory import (
    require_complete_tool_inventory,
)
from remote_mcp_server.simdorei_mcp.mcp_instructions import MCP_INSTRUCTIONS
from remote_mcp_server.simdorei_mcp.oauth_approval import create_approval_router
from remote_mcp_server.simdorei_mcp.oauth_provider import (
    SingleUserOAuthProvider,
    TokenCapacityError,
)
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    DEFAULT_OAUTH_SCOPES,
    OAUTH_SCOPES,
    READ_SCOPE,
)
from remote_mcp_server.simdorei_mcp.oauth_store import OAuthStore
from remote_mcp_server.simdorei_mcp.settings import GatewaySettings
from remote_mcp_server.simdorei_mcp.tools import (
    register_tools,
    registered_tool_names,
)


class GatewayConfigurationError(Exception):
    """Raised when a required public gateway setting is incomplete."""


class HealthResponse(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True)

    ok: bool
    service: str
    upstream_ready: bool
    configured_devices: int
    connected_devices: int


def create_app(settings: GatewaySettings) -> FastAPI:
    broker = BindingBroker(
        request_timeout_seconds=settings.request_timeout_seconds,
    )
    public_base_url = str(settings.public_base_url).rstrip("/")
    resource_url = f"{public_base_url}/mcp"
    oauth_provider = SingleUserOAuthProvider(
        store=OAuthStore(
            settings.oauth_database_path,
            max_clients=settings.oauth_client_limit,
            max_token_families=settings.oauth_token_family_global_limit,
            max_token_families_per_client=(
                settings.oauth_token_family_per_client_limit
            ),
            max_refresh_history_global=(
                settings.oauth_refresh_history_global_limit
            ),
            max_refresh_history_per_family=(
                settings.oauth_refresh_history_per_family_limit
            ),
        ),
        owner_token=settings.owner_token,
        public_base_url=public_base_url,
        resource_url=resource_url,
        access_token_seconds=settings.oauth_access_token_seconds,
        refresh_token_seconds=settings.oauth_refresh_token_seconds,
        pending_authorization_limit=(settings.oauth_pending_authorization_limit),
        authorization_code_limit=settings.oauth_authorization_code_global_limit,
        authorization_code_per_client_limit=(
            settings.oauth_authorization_code_per_client_limit
        ),
    )
    mcp = FastMCP(
        "simdorei-local-project",
        instructions=MCP_INSTRUCTIONS,
        auth_server_provider=oauth_provider,
        auth=AuthSettings(
            issuer_url=AnyHttpUrl(str(settings.public_base_url)),
            resource_server_url=AnyHttpUrl(resource_url),
            required_scopes=[READ_SCOPE],
            client_registration_options=ClientRegistrationOptions(
                enabled=True,
                valid_scopes=OAUTH_SCOPES,
                default_scopes=DEFAULT_OAUTH_SCOPES,
            ),
            revocation_options=RevocationOptions(enabled=True),
        ),
        json_response=True,
        stateless_http=True,
        transport_security=TransportSecuritySettings(
            allowed_hosts=[
                _public_host(settings),
                "localhost",
                "localhost:*",
                "127.0.0.1",
                "127.0.0.1:*",
            ],
            allowed_origins=[str(settings.public_base_url).rstrip("/")],
        ),
    )
    register_tools(mcp, broker, resource_url=resource_url)
    _ = require_complete_tool_inventory(registered_tool_names(mcp))
    mcp_app = mcp.streamable_http_app()
    mcp_app.add_exception_handler(TokenCapacityError, _token_capacity_error_response)

    @asynccontextmanager
    async def lifespan(_: FastAPI) -> AsyncGenerator[None, None]:
        async with _close_preserving_lifespan(
            mcp.session_manager.run(),
            oauth_provider.close,
        ):
            yield

    app = FastAPI(
        title="Simdorei Local Project MCP",
        lifespan=lifespan,
    )
    public_path = (settings.public_base_url.path or "").rstrip("/")
    approval_path = f"{public_path}/oauth/approve"
    app.include_router(
        create_approval_router(oauth_provider, approval_path=approval_path)
    )
    app.include_router(create_bridge_router(settings, broker))

    @app.get("/healthz")
    async def health() -> HealthResponse:
        connected_devices = await broker.connected_device_count()
        return HealthResponse(
            ok=True,
            service="simdorei-local-project-mcp",
            upstream_ready=connected_devices > 0,
            configured_devices=len(settings.device_credentials.devices),
            connected_devices=connected_devices,
        )

    _ = health

    app.mount("/", mcp_app)
    return app


@asynccontextmanager
async def _close_preserving_lifespan(
    session_context: AbstractAsyncContextManager[None],
    close_oauth: Callable[[], Awaitable[None]],
) -> AsyncGenerator[None, None]:
    try:
        async with session_context:
            yield
    except BaseException as session_error:
        try:
            await close_oauth()
        except BaseException as close_error:  # noqa: BLE001 - preserve both failures.
            raise BaseExceptionGroup(
                "MCP session and OAuth shutdown failures",
                (session_error, close_error),
            ) from None
        raise
    await close_oauth()


def _public_host(settings: GatewaySettings) -> str:
    host = settings.public_base_url.host
    if host is None:
        raise GatewayConfigurationError(
            "SIMDOREI_MCP_PUBLIC_BASE_URL must include a host."
        )
    return host


async def _token_capacity_error_response(
    _request: Request,
    _error: Exception,
) -> JSONResponse:
    return JSONResponse(
        {
            "error": "temporarily_unavailable",
            "error_description": "OAuth token storage capacity is full.",
        },
        status_code=503,
        headers={"Cache-Control": "no-store", "Pragma": "no-cache"},
    )
