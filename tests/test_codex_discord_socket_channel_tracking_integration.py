from __future__ import annotations

import unittest
from typing import Protocol, cast

import codex_discord_bot as bot
import codex_discord_bot_socket_runtime as discord_bot_socket_runtime


class TransientSocketChannelPermissionError(RuntimeError):
    pass


class BadSocketChannelDependencyError(TypeError):
    pass


class IsTrackedSocketMessageChannelFunc(Protocol):
    def __call__(
        self,
        client: discord_bot_socket_runtime.SocketRuntimeOwner,
        channel_id: int | None,
        /,
    ) -> tuple[bool, str]: ...


IS_TRACKED_SOCKET_MESSAGE_CHANNEL = cast(
    IsTrackedSocketMessageChannelFunc,
    bot.CodexDiscordBot.is_tracked_socket_message_channel,
)


class FakeSocketChannel:
    def __init__(self, channel_id: int) -> None:
        self.id: int = channel_id


class RuntimeFailureSocketClient:
    def is_allowed_channel(self, channel_id: int | None) -> bool:
        _ = channel_id
        return False

    def is_allowed_message_channel(self, channel: object) -> bool:
        _ = channel
        raise TransientSocketChannelPermissionError("transient cache check")

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[FakeSocketChannel, str]:
        return FakeSocketChannel(channel_id), "test_cache"


class TypeFailureSocketClient:
    def is_allowed_channel(self, channel_id: int | None) -> bool:
        _ = channel_id
        return False

    def is_allowed_message_channel(self, channel: object) -> bool:
        _ = channel
        raise BadSocketChannelDependencyError("bad tracked socket channel dependency")

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[FakeSocketChannel, str]:
        return FakeSocketChannel(channel_id), "test_cache"


class DiscordSocketChannelTrackingIntegrationTests(unittest.TestCase):
    def test_tracked_socket_channel_runtime_permission_failure_returns_cache_error(self) -> None:
        fake_client = RuntimeFailureSocketClient()

        tracked, source = IS_TRACKED_SOCKET_MESSAGE_CHANNEL(fake_client, 222)

        self.assertFalse(tracked)
        self.assertEqual(source, "cache_error")

    def test_tracked_socket_channel_type_error_is_not_cache_error(self) -> None:
        fake_client = TypeFailureSocketClient()

        with self.assertRaisesRegex(BadSocketChannelDependencyError, "bad tracked socket channel dependency"):
            _ = IS_TRACKED_SOCKET_MESSAGE_CHANNEL(fake_client, 222)


if __name__ == "__main__":
    _ = unittest.main()
