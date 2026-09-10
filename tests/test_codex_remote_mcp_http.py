from __future__ import annotations

import socket

import httpcore2
import pytest

from codex_remote_mcp_http import (
    PublicNetworkBackend,
    PublicNetworkError,
)


class RecordingBackend(httpcore2.NetworkBackend):
    def __init__(self) -> None:
        self.connected_hosts: list[str] = []

    def connect_tcp(
        self,
        host: str,
        port: int,
        timeout: float | None = None,
        local_address: str | None = None,
        socket_options=None,
    ) -> httpcore2.NetworkStream:
        self.connected_hosts.append(host)
        return httpcore2.NetworkStream()

    def connect_unix_socket(
        self,
        path: str,
        timeout: float | None = None,
        socket_options=None,
    ) -> httpcore2.NetworkStream:
        raise AssertionError("unexpected Unix socket")


def test_public_backend_connects_to_validated_ip_without_second_dns_lookup(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Given
    lookups: list[str] = []

    def resolve(host: str, *_args, **_kwargs):
        lookups.append(host)
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("93.184.216.34", 443))]

    delegate = RecordingBackend()
    monkeypatch.setattr("codex_remote_mcp_http.socket.getaddrinfo", resolve)

    # When
    _ = PublicNetworkBackend(delegate).connect_tcp("rebind.example", 443)

    # Then
    assert lookups == ["rebind.example"]
    assert delegate.connected_hosts == ["93.184.216.34"]


def test_public_backend_rejects_private_dns_answer(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Given
    monkeypatch.setattr(
        "codex_remote_mcp_http.socket.getaddrinfo",
        lambda *_args, **_kwargs: [
            (socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", 443))
        ],
    )
    delegate = RecordingBackend()

    # When / Then
    with pytest.raises(PublicNetworkError, match="public network"):
        PublicNetworkBackend(delegate).connect_tcp("private.example", 443)
    assert delegate.connected_hosts == []
