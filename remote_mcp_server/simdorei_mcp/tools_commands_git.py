from __future__ import annotations

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.oauth_scopes import READ_SCOPE, WRITE_SCOPE
from remote_mcp_server.simdorei_mcp.tool_context import (
    OPEN_WORLD_ANNOTATIONS,
    READ_AUTH_META,
    READ_ONLY_ANNOTATIONS,
    WRITE_ANNOTATIONS,
    WRITE_AUTH_META,
    ToolContext,
    execute_operation,
)
from simdorei_mcp_common.operation_outputs import (
    CommandListOutput,
    CommandRunOutput,
    GitCommitOutput,
    GitPushOutput,
    RepoDiffOutput,
    RepoStatusOutput,
)
from simdorei_mcp_common.operation_requests import (
    CommandListRequest,
    CommandRunRequest,
    GitCommitRequest,
    GitPushRequest,
    RepoDiffRequest,
    RepoStatusRequest,
)


def register_command_git_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="List safe project commands",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def command_list(ctx: ToolContext) -> CommandListOutput:
        """List manifest-defined checks that can run without arbitrary shell access."""
        return await execute_operation(
            ctx,
            broker,
            CommandListRequest(),
            CommandListOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Run safe project command",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def command_run(
        command_id: str,
        ctx: ToolContext,
        timeout_seconds: int = 60,
    ) -> CommandRunOutput:
        """Run one discovered non-network, non-destructive project check."""
        return await execute_operation(
            ctx,
            broker,
            CommandRunRequest(
                command_id=command_id,
                timeout_seconds=timeout_seconds,
            ),
            CommandRunOutput,
            required_scope=WRITE_SCOPE,
        )

    @mcp.tool(
        title="Show Git repository status",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def repo_status(ctx: ToolContext) -> RepoStatusOutput:
        """Show branch, changed files, remotes, and upstream distance."""
        return await execute_operation(
            ctx,
            broker,
            RepoStatusRequest(),
            RepoStatusOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Summarize Git changes",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def repo_diff_summary(ctx: ToolContext) -> RepoDiffOutput:
        """Return a bounded Git diff and per-file line counts."""
        return await execute_operation(
            ctx,
            broker,
            RepoDiffRequest(),
            RepoDiffOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Show project changes",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def show_changes(ctx: ToolContext) -> RepoDiffOutput:
        """Return the same bounded Git change report used for review."""
        return await execute_operation(
            ctx,
            broker,
            RepoDiffRequest(),
            RepoDiffOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Commit project changes",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def git_commit(
        message: str,
        paths: list[str],
        ctx: ToolContext,
    ) -> GitCommitOutput:
        """Create a Git commit from validated project paths."""
        return await execute_operation(
            ctx,
            broker,
            GitCommitRequest(message=message, paths=tuple(paths)),
            GitCommitOutput,
            required_scope=WRITE_SCOPE,
        )

    @mcp.tool(
        title="Push project branch",
        annotations=OPEN_WORLD_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def git_push(
        ctx: ToolContext,
        remote: str = "origin",
        branch: str | None = None,
    ) -> GitPushOutput:
        """Push the current or named branch to an already configured remote."""
        return await execute_operation(
            ctx,
            broker,
            GitPushRequest(remote=remote, branch=branch),
            GitPushOutput,
            required_scope=WRITE_SCOPE,
        )
