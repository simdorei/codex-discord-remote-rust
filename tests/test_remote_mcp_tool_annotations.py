from remote_mcp_server.simdorei_mcp.tool_context import SELECT_ANNOTATIONS


def test_select_project_is_declared_read_only() -> None:
    assert SELECT_ANNOTATIONS.readOnlyHint is True
    assert SELECT_ANNOTATIONS.destructiveHint is False
    assert SELECT_ANNOTATIONS.openWorldHint is False
