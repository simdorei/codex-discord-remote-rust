from __future__ import annotations

import ast
import inspect
import textwrap

from mcp.server.auth.provider import OAuthAuthorizationServerProvider
from typing_extensions import get_protocol_members

from remote_mcp_server.simdorei_mcp.oauth_provider import SingleUserOAuthProvider


def test_oauth_provider_implements_every_sdk_coroutine_as_override() -> None:
    # Given: the complete public contract exposed by the installed MCP SDK.
    sdk_methods = get_protocol_members(OAuthAuthorizationServerProvider)

    # When: directly declared async implementations marked with @override are selected.
    module = ast.parse(textwrap.dedent(inspect.getsource(SingleUserOAuthProvider)))
    provider_class = next(
        node for node in module.body if isinstance(node, ast.ClassDef)
    )
    marked_implementations = {
        node.name
        for node in provider_class.body
        if isinstance(node, ast.AsyncFunctionDef)
        and any(
            isinstance(decorator, ast.Name) and decorator.id == "override"
            for decorator in node.decorator_list
        )
    }

    # Then: no current or newly added SDK method can be inherited as a stub.
    assert sdk_methods <= marked_implementations
