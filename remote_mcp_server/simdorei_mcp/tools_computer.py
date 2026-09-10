from __future__ import annotations

from mcp.server.fastmcp import FastMCP
from mcp.types import CallToolResult, ImageContent, TextContent

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_OBSERVE_REQUIRED_SCOPES,
)
from remote_mcp_server.simdorei_mcp.tool_context import (
    COMPUTER_CONTROL_ANNOTATIONS,
    COMPUTER_CONTROL_AUTH_META,
    COMPUTER_OBSERVE_ANNOTATIONS,
    COMPUTER_OBSERVE_AUTH_META,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.operation_outputs import (
    ComputerActionOutput,
    ComputerScreenshotOutput,
    ComputerWindowsOutput,
)
from simdorei_mcp_common.operation_requests import (
    ComputerActivateRequest,
    ComputerApp,
    ComputerLaunchRequest,
    ComputerListWindowsRequest,
    ComputerScreenshotRequest,
    ComputerWindowId,
)


def register_computer_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="List controllable computer windows",
        annotations=COMPUTER_OBSERVE_ANNOTATIONS,
        meta=COMPUTER_OBSERVE_AUTH_META,
        structured_output=True,
    )
    async def list_computer_windows(ctx: ToolContext) -> ComputerWindowsOutput:
        """List session-owned windows in project mode or visible windows in PC mode."""
        return await execute_operation(
            ctx,
            broker,
            ComputerListWindowsRequest(),
            ComputerWindowsOutput,
            required_scope=COMPUTER_OBSERVE_REQUIRED_SCOPES,
        )

    @mcp.tool(
        title="Activate computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def activate_computer_window(
        window_id: ComputerWindowId,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Bring one window listed for the current project or PC selection forward."""
        from remote_mcp_server.simdorei_mcp.tools_computer_actions import run_action

        return await run_action(
            ctx,
            broker,
            ComputerActivateRequest(window_id=window_id),
        )

    @mcp.tool(
        title="Launch allowed computer app",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def launch_computer_app(
        app: ComputerApp,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Launch isolated Chrome or blank Notepad for this selected chat session."""
        from remote_mcp_server.simdorei_mcp.tools_computer_actions import run_action

        return await run_action(ctx, broker, ComputerLaunchRequest(app=app))

    @mcp.tool(
        title="Screenshot active computer window",
        annotations=COMPUTER_OBSERVE_ANNOTATIONS,
        meta=COMPUTER_OBSERVE_AUTH_META,
    )
    async def screenshot_computer_window(
        window_id: ComputerWindowId,
        ctx: ToolContext,
    ) -> CallToolResult:
        """Capture a permitted active window and issue one-use observation state."""
        result = await execute_operation(
            ctx,
            broker,
            ComputerScreenshotRequest(window_id=window_id),
            ComputerScreenshotOutput,
            required_scope=COMPUTER_OBSERVE_REQUIRED_SCOPES,
        )
        structured = result.model_dump(exclude={"data_base64"})
        return CallToolResult(
            content=[
                TextContent(
                    type="text",
                    text=(
                        "Use observation_id from structuredContent for exactly one "
                        "input action within 30 seconds."
                    ),
                ),
                ImageContent(
                    type="image",
                    data=result.data_base64,
                    mimeType=result.media_type,
                ),
            ],
            structuredContent=structured,
        )

    from remote_mcp_server.simdorei_mcp.tools_computer_actions import (
        register_computer_action_tools,
    )

    register_computer_action_tools(mcp, broker)
