from __future__ import annotations

import pytest

from codex_remote_mcp_bridge_config import (
    RemoteMcpConfigurationError,
    load_remote_mcp_config,
)


def _environment(bridge_url: str) -> dict[str, str]:
    return {
        "CODEX_REMOTE_MCP_ENABLED": "true",
        "CODEX_REMOTE_MCP_BRIDGE_URL": bridge_url,
        "CODEX_REMOTE_MCP_DEVICE_ID": "device-a",
        "CODEX_REMOTE_MCP_DEVICE_TOKEN": "device-secret",
    }


@pytest.mark.parametrize(
    "bridge_url",
    (
        "ws://localhost@evil.example/bridge",
        "ws://localhost.evil.example/bridge",
        "ws://127.0.0.1.evil.example/bridge",
        "ws://[::1]@evil.example/bridge",
    ),
)
def test_plaintext_bridge_rejects_deceptive_loopback_urls(bridge_url: str) -> None:
    with pytest.raises(RemoteMcpConfigurationError, match="must use wss"):
        _ = load_remote_mcp_config(_environment(bridge_url))


@pytest.mark.parametrize(
    "bridge_url",
    (
        "ws://localhost:8030/bridge",
        "ws://127.0.0.1:8030/bridge",
        "ws://[::1]:8030/bridge",
        "wss://simdorei.duckdns.org/bridge",
    ),
)
def test_bridge_accepts_encrypted_or_exact_loopback_urls(bridge_url: str) -> None:
    config = load_remote_mcp_config(_environment(bridge_url))

    assert config is not None
    assert config.bridge_url == bridge_url


@pytest.mark.parametrize(
    "bridge_url",
    (
        "wss://example.test/bridge#ignored",
        "ws://localhost:8030/bridge#ignored",
    ),
)
def test_remote_mcp_bridge_rejects_url_fragments(bridge_url: str) -> None:
    with pytest.raises(RemoteMcpConfigurationError, match="must use wss"):
        _ = load_remote_mcp_config(_environment(bridge_url))
