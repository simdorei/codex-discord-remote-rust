from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import (
    COMPUTER_CONTROL_REQUIRED_SCOPES,
)
from remote_mcp_server.simdorei_mcp.tool_context import (
    COMPUTER_CONTROL_ANNOTATIONS,
    COMPUTER_CONTROL_AUTH_META,
    COMPUTER_STOP_ANNOTATIONS,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.operation_outputs import (
    ComputerActionOutput,
    ComputerStopOutput,
)
from simdorei_mcp_common.operation_requests import (
    ComputerActivateRequest,
    ComputerClickCount,
    ComputerClickRequest,
    ComputerClipboardText,
    ComputerCloseRequest,
    ComputerCoordinate,
    ComputerDragRequest,
    ComputerKeyList,
    ComputerLaunchRequest,
    ComputerMouseButton,
    ComputerObservationId,
    ComputerPressKeysRequest,
    ComputerScrollDelta,
    ComputerScrollRequest,
    ComputerSetClipboardRequest,
    ComputerStopRequest,
    ComputerText,
    ComputerTypeTextRequest,
    ComputerWindowId,
)

ComputerActionRequest = (
    ComputerActivateRequest
    | ComputerLaunchRequest
    | ComputerClickRequest
    | ComputerDragRequest
    | ComputerScrollRequest
    | ComputerTypeTextRequest
    | ComputerPressKeysRequest
    | ComputerCloseRequest
    | ComputerSetClipboardRequest
)


def register_computer_action_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="Click computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def click_computer_window(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        x: ComputerCoordinate,
        y: ComputerCoordinate,
        ctx: ToolContext,
        button: ComputerMouseButton = "left",
        click_count: ComputerClickCount = 1,
    ) -> ComputerActionOutput:
        """Click screenshot-relative coordinates after a fresh screenshot."""
        return await run_action(
            ctx,
            broker,
            ComputerClickRequest(
                window_id=window_id,
                observation_id=observation_id,
                x=x,
                y=y,
                button=button,
                click_count=click_count,
            ),
        )

    @mcp.tool(
        title="Drag in computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def drag_computer_window(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        start_x: ComputerCoordinate,
        start_y: ComputerCoordinate,
        end_x: ComputerCoordinate,
        end_y: ComputerCoordinate,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Drag between screenshot-relative points after a fresh screenshot."""
        return await run_action(
            ctx,
            broker,
            ComputerDragRequest(
                window_id=window_id,
                observation_id=observation_id,
                start_x=start_x,
                start_y=start_y,
                end_x=end_x,
                end_y=end_y,
            ),
        )

    @mcp.tool(
        title="Scroll computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def scroll_computer_window(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        x: ComputerCoordinate,
        y: ComputerCoordinate,
        ctx: ToolContext,
        delta_x: ComputerScrollDelta = 0,
        delta_y: ComputerScrollDelta = 0,
    ) -> ComputerActionOutput:
        """Scroll at screenshot-relative coordinates after a fresh screenshot."""
        return await run_action(
            ctx,
            broker,
            ComputerScrollRequest(
                window_id=window_id,
                observation_id=observation_id,
                x=x,
                y=y,
                delta_x=delta_x,
                delta_y=delta_y,
            ),
        )

    @mcp.tool(
        title="Type text into computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def type_computer_text(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        text: ComputerText,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Type Unicode text after a fresh screenshot; never enter secrets or OTPs."""
        return await run_action(
            ctx,
            broker,
            ComputerTypeTextRequest(
                window_id=window_id,
                observation_id=observation_id,
                text=text,
            ),
        )

    @mcp.tool(
        title="Press computer keys",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def press_computer_keys(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        keys: ComputerKeyList,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Press a mode-permitted key chord after a fresh screenshot."""
        return await run_action(
            ctx,
            broker,
            ComputerPressKeysRequest(
                window_id=window_id,
                observation_id=observation_id,
                keys=tuple(keys),
            ),
        )

    @mcp.tool(
        title="Set computer clipboard text",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def set_computer_clipboard(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        text: ComputerClipboardText,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Replace clipboard text after observing a mode-permitted active window."""
        return await run_action(
            ctx,
            broker,
            ComputerSetClipboardRequest(
                window_id=window_id,
                observation_id=observation_id,
                text=text,
            ),
        )

    @mcp.tool(
        title="Close computer window",
        annotations=COMPUTER_CONTROL_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def close_computer_window(
        window_id: ComputerWindowId,
        observation_id: ComputerObservationId,
        ctx: ToolContext,
    ) -> ComputerActionOutput:
        """Request close after a fresh screenshot; save dialogs remain visible."""
        return await run_action(
            ctx,
            broker,
            ComputerCloseRequest(
                window_id=window_id,
                observation_id=observation_id,
            ),
        )

    @mcp.tool(
        title="Emergency stop computer control",
        annotations=COMPUTER_STOP_ANNOTATIONS,
        meta=COMPUTER_CONTROL_AUTH_META,
        structured_output=True,
    )
    async def stop_computer_control(ctx: ToolContext) -> ComputerStopOutput:
        """Stop this thread's computer control until the project is bound again."""
        return await execute_operation(
            ctx,
            broker,
            ComputerStopRequest(),
            ComputerStopOutput,
            required_scope=COMPUTER_CONTROL_REQUIRED_SCOPES,
        )


async def run_action(
    ctx: ToolContext,
    broker: BindingBroker,
    request: ComputerActionRequest,
) -> ComputerActionOutput:
    return await execute_operation(
        ctx,
        broker,
        request,
        ComputerActionOutput,
        required_scope=COMPUTER_CONTROL_REQUIRED_SCOPES,
    )
