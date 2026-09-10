from __future__ import annotations

from mcp.server.fastmcp import FastMCP
from mcp.server.fastmcp.exceptions import ToolError

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import BrokerError
from remote_mcp_server.simdorei_mcp.oauth_scopes import READ_SCOPE
from remote_mcp_server.simdorei_mcp.tool_context import (
    READ_AUTH_META,
    READ_ONLY_ANNOTATIONS,
    SELECT_ANNOTATIONS,
    ToolContext,
    tool_identity,
)
from simdorei_mcp_common.connector_contract import (
    PRODUCTION_CONNECTOR_NAME,
    PRODUCTION_CONNECTOR_RESOURCE,
)
from simdorei_mcp_common.device_messages import (
    DeviceListOutput,
    DeviceSelectionOutput,
)
from simdorei_mcp_common.messages import DeviceId


def register_device_tools(
    mcp: FastMCP,
    broker: BindingBroker,
    *,
    resource_url: str,
) -> None:
    @mcp.tool(
        title="List connected PCs",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def list_devices(
        ctx: ToolContext,
    ) -> DeviceListOutput:
        """List the VPS bridge devices currently available to this account."""
        _ = tool_identity(ctx, READ_SCOPE)
        return await broker.list_devices()

    @mcp.tool(
        title="Select connected PC",
        annotations=SELECT_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def select_device(
        device_id: DeviceId,
        working_directory: str,
        connector_resource: str,
        ctx: ToolContext,
    ) -> DeviceSelectionOutput:
        """Bind this ChatGPT conversation to one connected PC and folder."""
        if (
            resource_url != PRODUCTION_CONNECTOR_RESOURCE
            or connector_resource != PRODUCTION_CONNECTOR_RESOURCE
        ):
            raise ToolError(
                "OAuth connector mismatch: select "
                + f"{PRODUCTION_CONNECTOR_NAME!r} and retry."
            )
        identity = tool_identity(ctx, READ_SCOPE)
        try:
            return await broker.select_device(
                identity.session,
                identity.subject,
                device_id,
                working_directory=working_directory,
            )
        except BrokerError as exc:
            raise ToolError(str(exc)) from exc

    @mcp.tool(
        title="Change selected PC folder",
        annotations=SELECT_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def set_working_directory(
        working_directory: str,
        ctx: ToolContext,
    ) -> DeviceSelectionOutput:
        """Change the folder used by this conversation's selected PC."""
        identity = tool_identity(ctx, READ_SCOPE)
        try:
            return await broker.set_working_directory(
                identity.session,
                identity.subject,
                working_directory,
            )
        except BrokerError as exc:
            raise ToolError(str(exc)) from exc

    @mcp.tool(
        title="Show selected PC",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def device_info(
        ctx: ToolContext,
    ) -> DeviceSelectionOutput:
        """Show the PC and working folder selected by this conversation."""
        identity = tool_identity(ctx, READ_SCOPE)
        try:
            return await broker.device_info(identity.session, identity.subject)
        except BrokerError as exc:
            raise ToolError(str(exc)) from exc


__all__ = ["register_device_tools"]
