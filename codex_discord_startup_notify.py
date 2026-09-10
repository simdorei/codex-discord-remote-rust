from __future__ import annotations

import traceback
from collections.abc import Awaitable, Callable
from typing import Protocol, TypeVar

ClientChannelT = TypeVar("ClientChannelT", covariant=True)
ChannelT = TypeVar("ChannelT")
SendChannelT = TypeVar("SendChannelT", contravariant=True)
DeliveryExceptionTypes = type[BaseException] | tuple[type[BaseException], ...]


class StartupNotifyClient(Protocol[ClientChannelT]):
    def get_channel(self, channel_id: int, /) -> ClientChannelT | None: ...

    def fetch_channel(self, channel_id: int, /) -> Awaitable[ClientChannelT]: ...


class StartupNoticeSender(Protocol[SendChannelT]):
    async def __call__(self, channel: SendChannelT, text: str, *, context: str) -> int: ...


async def send_startup_notice_if_enabled(
    client: StartupNotifyClient[ChannelT],
    startup_channel_id: int | None,
    *,
    notify_enabled: Callable[[], bool],
    is_messageable: Callable[[ChannelT], bool],
    send_chunks: StartupNoticeSender[ChannelT],
    build_startup_notice: Callable[[], str],
    log: Callable[[str], None],
    delivery_exceptions: DeliveryExceptionTypes,
) -> None:
    if not notify_enabled() or startup_channel_id is None:
        return
    channel = client.get_channel(startup_channel_id)
    if channel is None:
        try:
            channel = await client.fetch_channel(startup_channel_id)
        except delivery_exceptions:
            log("startup_channel_fetch_failed\n" + traceback.format_exc())
            return
    if is_messageable(channel):
        try:
            _ = await send_chunks(channel, build_startup_notice(), context="startup_notify")
            log(f"startup_notify_sent channel={startup_channel_id}")
        except delivery_exceptions:
            log("startup_notify_failed\n" + traceback.format_exc())
    else:
        log(f"startup_notify_skipped channel={startup_channel_id} reason=not_messageable")
