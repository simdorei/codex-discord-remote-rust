from __future__ import annotations

import ipaddress
import socket
import ssl
from collections.abc import Iterable
from urllib.parse import urlsplit

import httpcore2
import httpx2


class PublicNetworkError(Exception):
    """Raised when an outbound image request is not safe for public access."""


class PublicNetworkBackend(httpcore2.NetworkBackend):
    """Resolve once, reject non-public answers, and connect to the checked IP."""

    def __init__(
        self,
        delegate: httpcore2.NetworkBackend | None = None,
    ) -> None:
        self._delegate = delegate or httpcore2.SyncBackend()

    def connect_tcp(
        self,
        host: str,
        port: int,
        timeout: float | None = None,
        local_address: str | None = None,
        socket_options: Iterable[httpcore2.SOCKET_OPTION] | None = None,
    ) -> httpcore2.NetworkStream:
        addresses = _resolve_public_addresses(host, port)
        last_error: Exception | None = None
        for address in addresses:
            try:
                return self._delegate.connect_tcp(
                    address,
                    port,
                    timeout=timeout,
                    local_address=local_address,
                    socket_options=socket_options,
                )
            except (OSError, httpcore2.ConnectError) as exc:
                last_error = exc
        if last_error is not None:
            raise last_error
        raise PublicNetworkError("image URL hostname has no public address")

    def connect_unix_socket(
        self,
        path: str,
        timeout: float | None = None,
        socket_options: Iterable[httpcore2.SOCKET_OPTION] | None = None,
    ) -> httpcore2.NetworkStream:
        raise PublicNetworkError("Unix sockets are not allowed for image URLs")


class _ResponseStream(httpx2.SyncByteStream):
    def __init__(self, stream: Iterable[bytes]) -> None:
        self._stream = stream

    def __iter__(self):
        yield from self._stream

    def close(self) -> None:
        close = getattr(self._stream, "close", None)
        if close is not None:
            close()


class PinnedPublicTransport(httpx2.BaseTransport):
    """HTTPX2 transport whose TCP peer is the exact validated public IP."""

    def __init__(self) -> None:
        self._pool = httpcore2.ConnectionPool(
            ssl_context=ssl.create_default_context(),
            max_connections=200,
            max_keepalive_connections=40,
            keepalive_expiry=30,
            http2=True,
            retries=3,
            network_backend=PublicNetworkBackend(),
        )

    def handle_request(self, request: httpx2.Request) -> httpx2.Response:
        response = self._pool.handle_request(
            httpcore2.Request(
                method=request.method,
                url=httpcore2.URL(
                    scheme=request.url.raw_scheme,
                    host=request.url.raw_host,
                    port=request.url.port,
                    target=request.url.raw_path,
                ),
                headers=request.headers.raw,
                content=request.stream,
                extensions=request.extensions,
            )
        )
        if not isinstance(response.stream, Iterable):
            response.close()
            raise TypeError("synchronous image response returned an async stream")
        return httpx2.Response(
            status_code=response.status,
            headers=response.headers,
            stream=_ResponseStream(response.stream),
            extensions=response.extensions,
            request=request,
        )

    def close(self) -> None:
        self._pool.close()


def public_http_client() -> httpx2.Client:
    """Build a no-proxy client that validates every redirect destination."""
    return httpx2.Client(
        transport=PinnedPublicTransport(),
        timeout=httpx2.Timeout(connect=5, read=30, write=10, pool=10),
        follow_redirects=True,
        max_redirects=5,
        event_hooks={"request": [validate_public_url_shape]},
        trust_env=False,
    )


def validate_public_url_shape(request: httpx2.Request | str) -> None:
    value = str(request.url) if isinstance(request, httpx2.Request) else request
    parsed = urlsplit(value)
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.port not in (None, 443)
    ):
        raise PublicNetworkError(
            "image URL must be public HTTPS on the standard port"
        )


def _resolve_public_addresses(host: str, port: int) -> tuple[str, ...]:
    try:
        answers = socket.getaddrinfo(
            host,
            port,
            type=socket.SOCK_STREAM,
        )
    except OSError as exc:
        raise PublicNetworkError(
            "image URL hostname could not be resolved"
        ) from exc
    addresses = tuple(
        dict.fromkeys(str(answer[4][0]) for answer in answers)
    )
    if not addresses or any(
        not ipaddress.ip_address(address).is_global
        for address in addresses
    ):
        raise PublicNetworkError("image URL must use the public network")
    return addresses
