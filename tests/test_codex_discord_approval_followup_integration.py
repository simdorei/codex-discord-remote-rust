from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import NoReturn, Protocol, cast, final
from unittest import mock

import codex_discord_bot as bot
from codex_discord_steering import NativeExactWatchTarget
from codex_thread_models import ThreadInfo


class ThreadUnavailableError(RuntimeError):
    pass


class UnexpectedFetchError(AssertionError):
    pass


class BadChannelFetchDependencyError(TypeError):
    pass


@final
class FakeTarget:
    def __init__(self, channel_id: int = 222) -> None:
        self.id = channel_id

    async def send(self, content: str, view: None = None) -> None:
        _ = content, view


class MessageableTarget(Protocol):
    async def send(self, content: str, view: None = None) -> None:
        ...


class FetchClient(Protocol):
    def fetch_channel(self, channel_id: int) -> Awaitable[MessageableTarget]:
        ...


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: MessageableTarget | None


@dataclass(frozen=True, slots=True)
class FakeClient:
    fetch_channel_func: Callable[[int], Awaitable[MessageableTarget]]

    def fetch_channel(self, channel_id: int) -> Awaitable[MessageableTarget]:
        return self.fetch_channel_func(channel_id)


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    channel: MessageableTarget | None
    message: FakeMessage | None
    channel_id: int
    client: FetchClient | None


ResolveApprovalFollowupChannel = Callable[[FakeInteraction], Awaitable[MessageableTarget | None]]


async def resolve_approval_followup_channel(interaction: FakeInteraction) -> MessageableTarget | None:
    resolver = cast(ResolveApprovalFollowupChannel, bot.resolve_approval_followup_channel)
    return await resolver(interaction)


@final
class ApprovalFollowupIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def test_post_approval_watch_requires_and_preserves_exact_active_turn(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            session_path.write_text("", encoding="utf-8")
            thread = ThreadInfo("thread-1", "Title", temp_dir, 1, str(session_path), "gpt", "high", 0)
            with (
                mock.patch.object(bot.BRIDGE_THREAD_STATE, "choose_thread", return_value=thread),
                mock.patch.object(bot.BRIDGE_THREAD_STATE, "get_thread_workspace_ref", return_value="project:1"),
                mock.patch.object(
                    bot.app_server_transport.DEFAULT_CLIENT,
                    "get_active_turn_id",
                    return_value="turn-42",
                ),
            ):
                result = bot.make_post_approval_watch_result("thread-1")

        self.assertIsNotNone(result)
        assert result is not None
        self.assertEqual(result.watch_target, NativeExactWatchTarget("turn-42"))

    def test_post_approval_watch_skips_when_exact_turn_is_unavailable(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            session_path.write_text("", encoding="utf-8")
            thread = ThreadInfo("thread-1", "Title", temp_dir, 1, str(session_path), "gpt", "high", 0)
            with (
                mock.patch.object(bot.BRIDGE_THREAD_STATE, "choose_thread", return_value=thread),
                mock.patch.object(
                    bot.app_server_transport.DEFAULT_CLIENT,
                    "get_active_turn_id",
                    return_value=None,
                ),
            ):
                result = bot.make_post_approval_watch_result("thread-1")

        self.assertIsNone(result)

    def test_make_post_approval_watch_result_returns_none_when_thread_unavailable(self) -> None:
        def raise_unavailable(thread_id: str, cwd: str | None = None) -> NoReturn:
            _ = thread_id, cwd
            raise ThreadUnavailableError("thread unavailable")

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with (
                mock.patch.object(bot.BRIDGE_THREAD_STATE, "choose_thread", side_effect=raise_unavailable),
                mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}),
            ):
                result = bot.make_post_approval_watch_result("thread-1")
            log_text = log_path.read_text(encoding="utf-8")

        self.assertIsNone(result)
        self.assertIn(
            "approval_followup_watch_unavailable target=thread-1 error_type=ThreadUnavailableError",
            log_text,
        )

    async def test_resolve_approval_followup_channel_prefers_interaction_channel(self) -> None:
        preferred = FakeTarget(111)
        fallback = FakeTarget(222)

        async def fetch_channel(channel_id: int) -> FakeTarget:
            raise UnexpectedFetchError(f"unexpected fetch for {channel_id}")

        interaction = FakeInteraction(
            channel=preferred,
            message=FakeMessage(channel=fallback),
            channel_id=333,
            client=FakeClient(fetch_channel_func=fetch_channel),
        )

        resolved = await resolve_approval_followup_channel(interaction)

        self.assertIs(resolved, preferred)

    async def test_resolve_approval_followup_channel_uses_message_channel(self) -> None:
        fallback = FakeTarget(222)

        async def fetch_channel(channel_id: int) -> FakeTarget:
            raise UnexpectedFetchError(f"unexpected fetch for {channel_id}")

        interaction = FakeInteraction(
            channel=None,
            message=FakeMessage(channel=fallback),
            channel_id=333,
            client=FakeClient(fetch_channel_func=fetch_channel),
        )

        resolved = await resolve_approval_followup_channel(interaction)

        self.assertIs(resolved, fallback)

    async def test_resolve_approval_followup_channel_fetches_channel(self) -> None:
        fetched = FakeTarget(333)
        fetched_ids: list[int] = []

        async def fetch_channel(channel_id: int) -> FakeTarget:
            fetched_ids.append(channel_id)
            return fetched

        interaction = FakeInteraction(
            channel=None,
            message=None,
            channel_id=333,
            client=FakeClient(fetch_channel_func=fetch_channel),
        )

        resolved = await resolve_approval_followup_channel(interaction)

        self.assertIs(resolved, fetched)
        self.assertEqual(fetched_ids, [333])

    async def test_resolve_approval_followup_channel_logs_fetch_failure(self) -> None:
        async def fetch_channel(channel_id: int) -> FakeTarget:
            raise ThreadUnavailableError(f"missing channel {channel_id}")

        interaction = FakeInteraction(
            channel=None,
            message=None,
            channel_id=333,
            client=FakeClient(fetch_channel_func=fetch_channel),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}):
                resolved = await resolve_approval_followup_channel(interaction)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertIsNone(resolved)
        self.assertIn("approval_followup_channel_fetch_failed target_channel=333", log_text)
        self.assertIn("error_type=ThreadUnavailableError", log_text)

    async def test_resolve_approval_followup_channel_type_error_is_not_fetch_failure(self) -> None:
        async def fetch_channel(channel_id: int) -> FakeTarget:
            raise BadChannelFetchDependencyError(f"bad channel fetch dependency {channel_id}")

        interaction = FakeInteraction(
            channel=None,
            message=None,
            channel_id=333,
            client=FakeClient(fetch_channel_func=fetch_channel),
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with mock.patch.dict(os.environ, {"CODEX_DISCORD_LOG_PATH": str(log_path)}):
                with self.assertRaisesRegex(TypeError, "bad channel fetch dependency 333"):
                    _ = await resolve_approval_followup_channel(interaction)
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""

        self.assertNotIn("approval_followup_channel_fetch_failed", log_text)


if __name__ == "__main__":
    _ = unittest.main()
