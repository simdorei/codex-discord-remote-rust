from __future__ import annotations

import os
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import override
from urllib.parse import urlsplit


@dataclass(frozen=True, slots=True)
class RemoteMcpConfigurationError(Exception):
    reason: str

    @override
    def __str__(self) -> str:
        return self.reason


@dataclass(frozen=True, slots=True)
class RemoteMcpBridgeConfig:
    bridge_url: str
    device_id: str
    device_token: str
    binding_ttl_seconds: int = 1_800
    binding_ack_timeout_seconds: float = 10.0
    reconnect_delay_seconds: float = 2.0


@dataclass(frozen=True, slots=True)
class ProjectTicket:
    project_scope: str
    expires_at: datetime


@dataclass(frozen=True, slots=True)
class DeviceTicket:
    device_id: str
    working_directory: Path


def load_remote_mcp_config(
    environ: Mapping[str, str] | None = None,
) -> RemoteMcpBridgeConfig | None:
    values = os.environ if environ is None else environ
    if values.get("CODEX_REMOTE_MCP_ENABLED", "").strip().casefold() not in {
        "1",
        "true",
        "yes",
        "on",
    }:
        return None
    bridge_url = _required(values, "CODEX_REMOTE_MCP_BRIDGE_URL")
    device_id = _required(values, "CODEX_REMOTE_MCP_DEVICE_ID")
    device_token = _required(values, "CODEX_REMOTE_MCP_DEVICE_TOKEN")
    if not _secure_bridge_url(bridge_url):
        raise RemoteMcpConfigurationError(
            "CODEX_REMOTE_MCP_BRIDGE_URL must use wss:// (ws:// is allowed only for localhost)."
        )
    return RemoteMcpBridgeConfig(
        bridge_url=bridge_url,
        device_id=device_id,
        device_token=device_token,
        binding_ttl_seconds=_bounded_int(
            values,
            "CODEX_REMOTE_MCP_BINDING_TTL_SECONDS",
            default=1_800,
            minimum=60,
            maximum=86_400,
        ),
    )


def _required(values: Mapping[str, str], name: str) -> str:
    value = values.get(name, "").strip()
    if not value:
        raise RemoteMcpConfigurationError(
            f"{name} is required when remote MCP is enabled."
        )
    return value


def _secure_bridge_url(url: str) -> bool:
    try:
        parsed = urlsplit(url)
        _ = parsed.port
    except ValueError:
        return False
    if (
        parsed.hostname is None
        or bool(parsed.fragment)
        or parsed.username is not None
        or parsed.password is not None
    ):
        return False
    if parsed.scheme == "wss":
        return True
    return parsed.scheme == "ws" and parsed.hostname.casefold() in {
        "127.0.0.1",
        "::1",
        "localhost",
    }


def _bounded_int(
    values: Mapping[str, str],
    name: str,
    *,
    default: int,
    minimum: int,
    maximum: int,
) -> int:
    raw = values.get(name, "").strip()
    if not raw:
        return default
    try:
        parsed = int(raw)
    except ValueError as exc:
        raise RemoteMcpConfigurationError(f"{name} must be an integer.") from exc
    if not minimum <= parsed <= maximum:
        raise RemoteMcpConfigurationError(
            f"{name} must be between {minimum} and {maximum}."
        )
    return parsed
