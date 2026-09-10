from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    TERMINAL_INTERACT_REQUIRED_SCOPES,
)
from remote_mcp_server.simdorei_mcp.tool_context import (
    OPEN_WORLD_ANNOTATIONS,
    TERMINAL_INTERACT_AUTH_META,
    TERMINAL_LOCAL_DESTRUCTIVE_ANNOTATIONS,
    TERMINAL_LOCAL_STATE_ANNOTATIONS,
    TERMINAL_OBSERVE_ANNOTATIONS,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.terminal_window_interaction_protocol import (
    TerminalWindowActionOutput,
    TerminalWindowActivateRequest,
    TerminalWindowCaptureOutput,
    TerminalWindowCaptureRequest,
    TerminalWindowInterruptRequest,
    TerminalWindowKeysRequest,
    TerminalWindowObservationId,
    TerminalWindowTypeRequest,
)
from simdorei_mcp_common.terminal_window_protocol import TerminalWindowId


def register_terminal_window_interaction_tools(
    mcp: FastMCP,
    broker: BindingBroker,
) -> None:
    @mcp.tool(
        title="Capture a session-owned terminal window",
        annotations=TERMINAL_OBSERVE_ANNOTATIONS,
        meta=TERMINAL_INTERACT_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_capture(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        ctx: ToolContext,
    ) -> TerminalWindowCaptureOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowCaptureRequest(terminal_window_id=terminal_window_id),
            TerminalWindowCaptureOutput,
            required_scope=TERMINAL_INTERACT_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Activate a session-owned terminal window",
        annotations=TERMINAL_LOCAL_STATE_ANNOTATIONS,
        meta=TERMINAL_INTERACT_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_activate(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        ctx: ToolContext,
    ) -> TerminalWindowActionOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowActivateRequest(terminal_window_id=terminal_window_id),
            TerminalWindowActionOutput,
            required_scope=TERMINAL_INTERACT_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Type text into a captured session-owned terminal window",
        annotations=OPEN_WORLD_ANNOTATIONS,
        meta=TERMINAL_INTERACT_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_type(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
        text: str,
        ctx: ToolContext,
    ) -> TerminalWindowActionOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowTypeRequest(
                terminal_window_id=terminal_window_id,
                observation_id=observation_id,
                text=text,
            ),
            TerminalWindowActionOutput,
            required_scope=TERMINAL_INTERACT_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Press keys in a captured session-owned terminal window",
        annotations=OPEN_WORLD_ANNOTATIONS,
        meta=TERMINAL_INTERACT_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_keys(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
        keys: tuple[str, ...],
        ctx: ToolContext,
    ) -> TerminalWindowActionOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowKeysRequest(
                terminal_window_id=terminal_window_id,
                observation_id=observation_id,
                keys=keys,
            ),
            TerminalWindowActionOutput,
            required_scope=TERMINAL_INTERACT_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Interrupt a captured session-owned terminal process",
        annotations=TERMINAL_LOCAL_DESTRUCTIVE_ANNOTATIONS,
        meta=TERMINAL_INTERACT_AUTH_META,
        structured_output=True,
    )
    async def terminal_window_interrupt(  # pyright: ignore[reportUnusedFunction]
        terminal_window_id: TerminalWindowId,
        observation_id: TerminalWindowObservationId,
        ctx: ToolContext,
    ) -> TerminalWindowActionOutput:
        return await execute_operation(
            ctx,
            broker,
            TerminalWindowInterruptRequest(
                terminal_window_id=terminal_window_id,
                observation_id=observation_id,
            ),
            TerminalWindowActionOutput,
            required_scope=TERMINAL_INTERACT_REQUIRED_SCOPES,
        )


__all__ = ["register_terminal_window_interaction_tools"]
