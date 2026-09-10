from __future__ import annotations

import base64

from mcp.server.fastmcp import FastMCP, Image
from pydantic import HttpUrl

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
    ImageListOutput,
    ImageRetrieveOutput,
    ImageSaveOutput,
)
from simdorei_mcp_common.operation_requests import (
    ListImagesRequest,
    RetrieveImageRequest,
    SaveImageFromUrlRequest,
    SaveImageRequest,
)


def register_image_tools(mcp: FastMCP, broker: BindingBroker) -> None:
    @mcp.tool(
        title="Save project image",
        annotations=WRITE_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def save_image(
        path: str,
        data_base64: str,
        ctx: ToolContext,
        overwrite: bool = False,
    ) -> ImageSaveOutput:
        """Save a validated base64 PNG, JPEG, GIF, or WebP project image."""
        return await execute_operation(
            ctx,
            broker,
            SaveImageRequest(
                path=path,
                data_base64=data_base64,
                overwrite=overwrite,
            ),
            ImageSaveOutput,
            required_scope=WRITE_SCOPE,
        )

    @mcp.tool(
        title="Save image from public URL",
        annotations=OPEN_WORLD_ANNOTATIONS,
        meta=WRITE_AUTH_META,
        structured_output=True,
    )
    async def save_image_from_url(
        path: str,
        url: str,
        ctx: ToolContext,
        overwrite: bool = False,
    ) -> ImageSaveOutput:
        """Download a public HTTPS image into the selected project."""
        return await execute_operation(
            ctx,
            broker,
            SaveImageFromUrlRequest(
                path=path,
                url=HttpUrl(url),
                overwrite=overwrite,
            ),
            ImageSaveOutput,
            required_scope=WRITE_SCOPE,
        )

    @mcp.tool(
        title="List project images",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
        structured_output=True,
    )
    async def list_images(ctx: ToolContext) -> ImageListOutput:
        """List supported image files inside the selected project."""
        return await execute_operation(
            ctx,
            broker,
            ListImagesRequest(),
            ImageListOutput,
            required_scope=READ_SCOPE,
        )

    @mcp.tool(
        title="Retrieve project image",
        annotations=READ_ONLY_ANNOTATIONS,
        meta=READ_AUTH_META,
    )
    async def retrieve_image(
        path: str,
        ctx: ToolContext,
    ) -> Image:
        """Return one project image as native MCP image content for vision."""
        retrieved = await execute_operation(
            ctx,
            broker,
            RetrieveImageRequest(path=path),
            ImageRetrieveOutput,
            required_scope=READ_SCOPE,
        )
        image_format = retrieved.image.media_type.removeprefix("image/")
        return Image(
            data=base64.b64decode(retrieved.data_base64, validate=True),
            format=image_format,
        )
