from __future__ import annotations

import os
import tempfile
import unittest
from collections.abc import Awaitable
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol, TypeAlias, cast, final, override

import codex_discord_bot as bot
import codex_discord_text as discord_text


class FollowupUnavailableError(RuntimeError):
    pass


class UnexpectedViewKeywordError(TypeError):
    pass


@final
class FakeView:
    pass


FollowupKwargValue: TypeAlias = bool | FakeView


@final
class RecordingFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, FollowupKwargValue]] = []

    async def send(
        self,
        content: str,
        view: FakeView | None = None,
        **kwargs: FollowupKwargValue,
    ) -> None:
        _ = view
        self.messages.append(content)
        self.kwargs.append(kwargs)


@final
class FailingFollowup:
    def __init__(self, fail_after: int = 0) -> None:
        self.messages: list[str] = []
        self.fail_after = fail_after

    async def send(
        self,
        content: str,
        view: FakeView | None = None,
        **kwargs: FollowupKwargValue,
    ) -> None:
        _ = content, view, kwargs
        if len(self.messages) >= self.fail_after:
            raise FollowupUnavailableError("followup unavailable")
        self.messages.append(content)


@final
class NoViewKeywordFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, FollowupKwargValue]] = []

    async def send(self, content: str, **kwargs: FollowupKwargValue) -> None:
        if "view" in kwargs:
            raise UnexpectedViewKeywordError("send() got an unexpected keyword argument 'view'")
        self.messages.append(content)
        self.kwargs.append(kwargs)


@final
class RecordingTarget:
    def __init__(self) -> None:
        self.messages: list[tuple[str, FakeView | None]] = []

    async def send(self, content: str, view: FakeView | None = None) -> None:
        self.messages.append((content, view))


@dataclass(frozen=True, slots=True)
class FakeCommand:
    name: str


DeliveryFollowup: TypeAlias = RecordingFollowup | FailingFollowup | NoViewKeywordFollowup


@final
class FakeInteraction:
    def __init__(self, command_name: str = "help", channel_id: int = 12345, *, followup: DeliveryFollowup) -> None:
        self.command = FakeCommand(command_name)
        self.channel_id = channel_id
        self.followup = followup
        self.channel = RecordingTarget()


class SendFollowupChunksFunc(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        text: str,
        *,
        title: str,
        exit_code: int | None = None,
        log_prefix: str = "followup_response",
    ) -> Awaitable[None]:
        ...


class SendDirectFollowupFunc(Protocol):
    def __call__(
        self,
        interaction: FakeInteraction,
        content: str,
        *,
        view: FakeView | None = None,
        log_prefix: str = "direct_followup",
        context: str = "",
    ) -> Awaitable[None]:
        ...


async def send_followup_chunks(
    interaction: FakeInteraction,
    text: str,
    *,
    title: str,
    exit_code: int | None = None,
    log_prefix: str = "followup_response",
) -> None:
    sender = cast(SendFollowupChunksFunc, bot.send_followup_chunks)
    await sender(interaction, text, title=title, exit_code=exit_code, log_prefix=log_prefix)


async def send_direct_followup(
    interaction: FakeInteraction,
    content: str,
    *,
    view: FakeView | None = None,
    log_prefix: str = "direct_followup",
    context: str = "",
) -> None:
    sender = cast(SendDirectFollowupFunc, bot.send_direct_followup)
    await sender(interaction, content, view=view, log_prefix=log_prefix, context=context)


@final
class DiscordFollowupDeliveryIntegrationTests(unittest.IsolatedAsyncioTestCase):
    _old_discord_log_path: str | None = None
    _temp_dir: tempfile.TemporaryDirectory[str] | None = None

    @override
    def setUp(self) -> None:
        self._old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self._temp_dir = temp_dir
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(Path(temp_dir.name) / "discord-smoke.log")
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()

    @override
    def tearDown(self) -> None:
        if self._old_discord_log_path is None:
            _ = os.environ.pop("CODEX_DISCORD_LOG_PATH", None)
        else:
            os.environ["CODEX_DISCORD_LOG_PATH"] = self._old_discord_log_path
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        bot.clear_discord_delivery_stopping()
        if self._temp_dir is not None:
            self._temp_dir.cleanup()
            self._temp_dir = None

    def _log_text(self) -> str:
        return Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")

    async def test_send_followup_chunks_splits_long_button_response(self) -> None:
        followup = RecordingFollowup()
        interaction = FakeInteraction(command_name="ask", channel_id=222, followup=followup)

        await send_followup_chunks(
            interaction,
            "x" * 4100,
            title="Steering",
            exit_code=1,
            log_prefix="button_response",
        )

        log_text = self._log_text()
        self.assertGreater(len(followup.messages), 1)
        self.assertTrue(all(len(message) <= discord_text.DISCORD_MAX_LEN for message in followup.messages))
        self.assertIn("button_response_start command=ask title='Steering' exit=1", log_text)
        self.assertIn("button_response_sent command=ask title='Steering' exit=1", log_text)
        self.assertEqual(bot.ACTIVE_DISCORD_DELIVERIES, set())

    async def test_send_followup_chunks_surfaces_send_failure(self) -> None:
        followup = FailingFollowup()
        interaction = FakeInteraction(command_name="ask", channel_id=222, followup=followup)

        with self.assertRaisesRegex(RuntimeError, "followup unavailable"):
            await send_followup_chunks(
                interaction,
                "button result",
                title="Steering",
                exit_code=1,
                log_prefix="button_response",
            )

        log_text = self._log_text()
        self.assertEqual(followup.messages, [])
        self.assertEqual(interaction.channel.messages, [])
        self.assertIn("button_response_failed command=ask title='Steering' exit=1", log_text)
        self.assertIn("error_type=FollowupUnavailableError", log_text)
        self.assertIn("error=followup unavailable", log_text)
        self.assertNotIn("button_response_fallback_sent", log_text)
        self.assertEqual(bot.ACTIVE_DISCORD_DELIVERIES, set())

    async def test_send_followup_chunks_surfaces_partial_send_failure(self) -> None:
        followup = FailingFollowup(fail_after=1)
        interaction = FakeInteraction(command_name="ask", channel_id=222, followup=followup)

        with self.assertRaisesRegex(RuntimeError, "followup unavailable"):
            await send_followup_chunks(
                interaction,
                "x" * 4100,
                title="Steering",
                exit_code=1,
                log_prefix="button_response",
            )

        log_text = self._log_text()
        self.assertEqual(len(followup.messages), 1)
        self.assertEqual(interaction.channel.messages, [])
        self.assertIn("button_response_failed command=ask title='Steering' exit=1", log_text)
        self.assertIn("sent=1", log_text)
        self.assertIn("error=followup unavailable", log_text)
        self.assertNotIn("button_response_fallback_sent", log_text)
        self.assertEqual(bot.ACTIVE_DISCORD_DELIVERIES, set())

    async def test_send_direct_followup_surfaces_send_failure_with_view(self) -> None:
        followup = FailingFollowup()
        interaction = FakeInteraction(command_name="ask", channel_id=222, followup=followup)
        view = FakeView()

        with self.assertRaisesRegex(RuntimeError, "followup unavailable"):
            await send_direct_followup(
                interaction,
                "button view",
                view=view,
                log_prefix="button_followup",
                context="steer_busy_failure",
            )

        log_text = self._log_text()
        self.assertEqual(followup.messages, [])
        self.assertEqual(interaction.channel.messages, [])
        self.assertIn("button_followup_failed command=ask context=steer_busy_failure", log_text)
        self.assertIn("error=followup unavailable", log_text)
        self.assertNotIn("button_followup_fallback_sent", log_text)
        self.assertEqual(bot.ACTIVE_DISCORD_DELIVERIES, set())

    async def test_send_direct_followup_omits_empty_view_keyword(self) -> None:
        followup = NoViewKeywordFollowup()
        interaction = FakeInteraction(command_name="ask", channel_id=222, followup=followup)

        await send_direct_followup(
            interaction,
            "ignored",
            log_prefix="button_followup",
            context="ignore",
        )

        log_text = self._log_text()
        self.assertEqual(followup.messages, ["ignored"])
        self.assertEqual(followup.kwargs, [{}])
        self.assertEqual(interaction.channel.messages, [])
        self.assertIn("button_followup_sent command=ask context=ignore", log_text)
        self.assertNotIn("button_followup_failed command=ask context=ignore", log_text)
        self.assertEqual(bot.ACTIVE_DISCORD_DELIVERIES, set())


if __name__ == "__main__":
    _ = unittest.main()
