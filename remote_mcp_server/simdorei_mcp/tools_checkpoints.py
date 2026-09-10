from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import READ_SCOPE, WRITE_SCOPE
from remote_mcp_server.simdorei_mcp.tool_context import (
    READ_AUTH_META,
    READ_ONLY_ANNOTATIONS,
    WRITE_ANNOTATIONS,
    WRITE_AUTH_META,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.operation_outputs import (
    CheckpointListOutput,
    CheckpointRestoreOutput,
    CheckpointShowOutput,
)
from simdorei_mcp_common.operation_requests import (
    CheckpointListRequest,
    CheckpointRestoreRequest,
    CheckpointShowRequest,
)


def register_checkpoint_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="List project checkpoints",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def checkpoint_list(ctx: ToolContext) -> CheckpointListOutput:
        """List restorable checkpoints created by remote file mutations."""
        return await execute_operation(
            ctx,
            broker,
            CheckpointListRequest(),
            CheckpointListOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Show project checkpoint",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def checkpoint_show(
        checkpoint_id: str,
        ctx: ToolContext,
    ) -> CheckpointShowOutput:
        """Show the bounded change recorded in one checkpoint."""
        return await execute_operation(
            ctx,
            broker,
            CheckpointShowRequest(checkpoint_id=checkpoint_id),
            CheckpointShowOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Restore project checkpoint",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def checkpoint_restore(
        checkpoint_id: str,
        ctx: ToolContext,
    ) -> CheckpointRestoreOutput:
        """Restore project files to their state before one remote mutation."""
        return await execute_operation(
            ctx,
            broker,
            CheckpointRestoreRequest(checkpoint_id=checkpoint_id),
            CheckpointRestoreOutput,
            required_scope=WRITE_SCOPE,
        )
