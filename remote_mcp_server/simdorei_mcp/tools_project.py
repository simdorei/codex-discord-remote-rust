from __future__ import annotations

from mcp.server.fastmcp import FastMCP
from mcp.server.fastmcp.exceptions import ToolError

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.broker_errors import BrokerError
from remote_mcp_server.simdorei_mcp.oauth_scopes import READ_SCOPE, WRITE_SCOPE
from remote_mcp_server.simdorei_mcp.tool_context import (
    READ_AUTH_META,
    READ_ONLY_ANNOTATIONS,
    WRITE_ANNOTATIONS,
    WRITE_AUTH_META,
    ToolContext,
    execute_operation,
    tool_identity,
    tool_request_id,
)
from simdorei_mcp_common.messages import ReadFileCommand, ReadFileOutput
from simdorei_mcp_common.operation_outputs import (
    CodeSearchOutput,
    FileApplyPatchOutput,
    FileCreateOutput,
    ProjectRulesOutput,
    ProjectStatusOutput,
)
from simdorei_mcp_common.operation_requests import (
    CodeSearchRequest,
    FileApplyPatchRequest,
    FileChange,
    FileCreateRequest,
    ProjectRulesRequest,
    ProjectStatusRequest,
)


def register_project_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="Read project rules",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def project_rules(ctx: ToolContext) -> ProjectRulesOutput:
        """Read bounded AGENTS.md, CLAUDE.md, and Codex project rules."""
        return await execute_operation(
            ctx,
            broker,
            ProjectRulesRequest(),
            ProjectRulesOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Show project status",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def project_status(ctx: ToolContext) -> ProjectStatusOutput:
        """Show Git state, project rules, and safe commands at a glance."""
        return await execute_operation(
            ctx,
            broker,
            ProjectStatusRequest(),
            ProjectStatusOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Search project code",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def code_search(
        query: str,
        ctx: ToolContext,
        max_results: int = 100,
    ) -> CodeSearchOutput:
        """Search text in non-sensitive UTF-8 project files."""
        return await execute_operation(
            ctx,
            broker,
            CodeSearchRequest(query=query, max_results=max_results),
            CodeSearchOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Read project file slice",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def file_read_slice(
        path: str,
        ctx: ToolContext,
        start_line: int = 1,
        max_lines: int = 250,
    ) -> ReadFileOutput:
        """Read a bounded UTF-8 line range from one project file."""
        identity = tool_identity(ctx, READ_SCOPE)
        request_id = tool_request_id(ctx, identity)
        try:
            return await broker.read_file(
                identity.session,
                identity.subject,
                ReadFileCommand(
                    request_id=request_id,
                    thread_id="pending-route",
                    path=path,
                    start_line=start_line,
                    max_lines=max_lines,
                ),
            )
        except BrokerError as exc:
            raise ToolError(str(exc)) from exc

    @mcp.tool(
        title="Create project file",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def file_create(
        path: str,
        content: str,
        ctx: ToolContext,
        overwrite: bool = False,
    ) -> FileCreateOutput:
        """Create a UTF-8 file and record a restorable checkpoint."""
        return await execute_operation(
            ctx,
            broker,
            FileCreateRequest(path=path, content=content, overwrite=overwrite),
            FileCreateOutput,
            required_scope=WRITE_SCOPE,
        )

    @mcp.tool(
        title="Apply selected project changes",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def file_apply_patch(
        changes: list[FileChange],
        ctx: ToolContext,
    ) -> FileApplyPatchOutput:
        """Apply text file changes with hash checks and a restorable checkpoint."""
        return await execute_operation(
            ctx,
            broker,
            FileApplyPatchRequest(changes=tuple(changes)),
            FileApplyPatchOutput,
            required_scope=WRITE_SCOPE,
        )
