from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    TERMINAL_EXECUTE_REQUIRED_SCOPES,
)
from remote_mcp_server.simdorei_mcp.tool_context import (
    OPEN_WORLD_ANNOTATIONS,
    TERMINAL_EXECUTE_AUTH_META,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.terminal_protocol import (
    TerminalExecOutput,
    TerminalExecRequest,
    TerminalShell,
)


def register_terminal_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="Run arbitrary local terminal command",
        annotations=OPEN_WORLD_ANNOTATIONS,
        meta=TERMINAL_EXECUTE_AUTH_META,
        structured_output=True,
    )
    async def terminal_exec(  # pyright: ignore[reportUnusedFunction]
        command: str,
        ctx: ToolContext,
        terminal_id: str | None = None,
        shell: TerminalShell = "auto",
        cwd: str | None = None,
        environment: dict[str, str] | None = None,
        timeout_seconds: int = 300,
        cancel_previous: bool = False,
    ) -> TerminalExecOutput:
        """Run unrestricted shell text in a terminal owned by this chat session."""
        return await execute_operation(
            ctx,
            broker,
            TerminalExecRequest(
                terminal_id=terminal_id,
                shell=shell,
                command=command,
                cwd=cwd,
                environment=environment or {},
                timeout_seconds=timeout_seconds,
                cancel_previous=cancel_previous,
            ),
            TerminalExecOutput,
            required_scope=TERMINAL_EXECUTE_REQUIRED_SCOPES,
        )
