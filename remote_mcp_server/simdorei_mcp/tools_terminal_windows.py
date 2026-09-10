from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    TERMINAL_EXECUTE_REQUIRED_SCOPES,
)
from remote_mcp_server.simdorei_mcp.tool_context import (
    TERMINAL_EXECUTE_AUTH_META,
    TERMINAL_LOCAL_DESTRUCTIVE_ANNOTATIONS,
    TERMINAL_LOCAL_STATE_ANNOTATIONS,
    TERMINAL_OBSERVE_ANNOTATIONS,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.terminal_window_protocol import (
    TerminalWindowCloseOutput,
    TerminalWindowCloseRequest,
    TerminalWindowId,
    TerminalWindowListOutput,
    TerminalWindowListRequest,
    TerminalWindowOpenOutput,
    TerminalWindowOpenRequest,
    TerminalWindowShell,
)


def register_terminal_window_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="Open a session-owned terminal window",
        annotations=TERMINAL_LOCAL_STATE_ANNOTATIONS,
        meta=TERMINAL_EXECUTE_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_open(  # pyright: ignore[reportUnusedFunction]
        ctx: ToolContext,
        shell: TerminalWindowShell = "powershell",
        cwd: str | None = None,
    ) -> TerminalWindowOpenOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowOpenRequest(shell=shell, cwd=cwd),
            TerminalWindowOpenOutput,
            required_scope=TERMINAL_EXECUTE_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="List session-owned terminal windows",
        annotations=TERMINAL_OBSERVE_ANNOTATIONS,
        meta=TERMINAL_EXECUTE_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_list(  # pyright: ignore[reportUnusedFunction]
        ctx: ToolContext,
    ) -> TerminalWindowListOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowListRequest(),
            TerminalWindowListOutput,
            required_scope=TERMINAL_EXECUTE_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Close a session-owned terminal window",
        annotations=TERMINAL_LOCAL_DESTRUCTIVE_ANNOTATIONS,
        meta=TERMINAL_EXECUTE_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_close(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        ctx: ToolContext,
    ) -> TerminalWindowCloseOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowCloseRequest(terminal_window_id=terminal_window_id),
            TerminalWindowCloseOutput,
            required_scope=TERMINAL_EXECUTE_REQUIRED_SCOPES,
        )
