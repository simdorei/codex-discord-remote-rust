from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

from remote_mcp_server.simdorei_mcp.broker import BindingBroker
from remote_mcp_server.simdorei_mcp.tools import register_tools

INVENTORY_PATH = (
    Path(__file__).parents[1]
    / "crates"
    / "cdr-mcp-server"
    / "assets"
    / "python_tool_inventory.json"
)


def _serialized_tool(tool: Any) -> dict[str, Any]:
    item: dict[str, Any] = {
        "name": tool.name,
        "inputSchema": tool.parameters,
    }
    if tool.title is not None:
        item["title"] = tool.title
    if tool.description is not None:
        item["description"] = tool.description
    if tool.output_schema is not None:
        item["outputSchema"] = tool.output_schema
    if tool.annotations is not None:
        item["annotations"] = tool.annotations.model_dump(
            mode="json",
            by_alias=True,
            exclude_none=True,
        )
    if tool.meta is not None:
        item["_meta"] = tool.meta
    return item


def test_rust_mcp_inventory_matches_python_registration() -> None:
    mcp = FastMCP("inventory-contract")
    register_tools(
        mcp,
        BindingBroker(),
        resource_url="https://example.test/mcp",
    )
    actual = [
        _serialized_tool(tool)
        for tool in sorted(
            mcp._tool_manager.list_tools(),  # pyright: ignore[reportPrivateUsage]
            key=lambda value: value.name,
        )
    ]
    expected = json.loads(INVENTORY_PATH.read_text(encoding="utf-8"))

    assert actual == expected
