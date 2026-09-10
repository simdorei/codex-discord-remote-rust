from __future__ import annotations

from collections.abc import Awaitable, Callable
from contextlib import AbstractAsyncContextManager
from types import ModuleType
from typing import TypeVar, cast

import codex_app_server_transport as app_server_transport
import codex_discord_app_server_admission as admission


ChannelT = TypeVar("ChannelT")


def make_prompt_admission(
    module: ModuleType,
    send_notice: Callable[..., Awaitable[object]],
) -> Callable[[ChannelT, object | None, int | None], AbstractAsyncContextManager[bool]]:
    def prompt_admission(
        channel: ChannelT,
        source_message: object | None,
        expected_generation: int | None,
    ) -> AbstractAsyncContextManager[bool]:
        async def send(target: ChannelT, text: str) -> object:
            return await send_notice(target, text, context="app_server_prompt_discarded")

        enabled = cast(Callable[[], bool], getattr(module, "app_server_transport_enabled"))()
        log = cast(Callable[[str], None], getattr(module, "log_line"))
        return admission.admit_prompt_delivery(
            channel,
            source_message,
            expected_generation=expected_generation,
            transport_enabled=enabled,
            client=app_server_transport.DEFAULT_CLIENT,
            send_notice=send,
            log=log,
        )

    return prompt_admission
