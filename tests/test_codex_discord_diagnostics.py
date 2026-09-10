from __future__ import annotations

import datetime
import unittest
from collections.abc import AsyncIterator
from dataclasses import dataclass, field

import codex_discord_diagnostics as diagnostics


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int
    bot: bool


@dataclass(frozen=True, slots=True)
class FakeMessageType:
    name: str


@dataclass(frozen=True, slots=True)
class FakeHistoryMessage:
    created_at: datetime.datetime
    author: FakeAuthor
    content: str
    type: FakeMessageType


@dataclass(frozen=True, slots=True)
class FakeIntents:
    message_content: bool


@dataclass(frozen=True, slots=True)
class FakeDoneTask:
    done_result: bool

    def done(self) -> bool:
        return self.done_result


@dataclass(frozen=True, slots=True)
class FakeDoctorBot:
    enable_prefix_commands: bool = True
    intents: FakeIntents = FakeIntents(message_content=True)
    history_poll_seconds: int = 30
    startup_channel_id: int | None = 111
    allowed_channel_ids: set[int] = field(default_factory=lambda: {222, 333})
    allowed_user_ids: set[int] = field(default_factory=lambda: {444})
    _history_poll_task: FakeDoneTask | None = FakeDoneTask(done_result=False)
    _history_poll_last_at: str = "last-at"
    _history_poll_primed_channels: set[int] = field(default_factory=lambda: {222})
    _slash_sync_status: str = "ok"
    _slash_sync_last_at: str = "sync-at"
    _slash_sync_commands: int = 7
    _enable_debug_events: bool = True


class MirrorCheckError(RuntimeError):
    pass


class FetchChannelError(RuntimeError):
    pass


class DiscordDoctorLogMarkerTests(unittest.TestCase):
    def test_doctor_log_marker_lines_preserve_order(self) -> None:
        markers = {
            "last_ready_at": "ready",
            "last_gateway_event_at": "gateway",
            "last_raw_interaction_at": "raw",
            "last_interaction_at": "interaction",
            "last_component_at": "component",
            "last_user_or_control_hook_at": "user",
            "last_button_qa_at": "button",
            "last_button_qa_result": "ok",
            "last_steering_button_at": "steer",
            "last_steering_button_exit": "0",
            "last_steering_button_elapsed_sec": "1.2",
        }

        self.assertEqual(
            diagnostics.build_discord_doctor_log_marker_lines(markers),
            [
                "last_ready_at: ready",
                "last_gateway_event_at: gateway",
                "last_raw_interaction_at: raw",
                "last_interaction_at: interaction",
                "last_component_at: component",
                "last_user_or_control_hook_at: user",
                "last_button_qa_at: button",
                "last_button_qa_result: ok",
                "last_steering_button_at: steer",
                "last_steering_button_exit: 0",
                "last_steering_button_elapsed_sec: 1.2",
            ],
        )


class DiscordDoctorMessageTests(unittest.TestCase):
    def test_doctor_message_includes_state_and_mirror_failure(self) -> None:
        markers = {key: f"marker-{key}" for key in diagnostics.DISCORD_DOCTOR_LOG_MARKER_KEYS}

        def build_mirror_check() -> str:
            raise MirrorCheckError("mirror broke")

        def recent_events(*, user_or_control_only: bool = False) -> list[str]:
            if user_or_control_only:
                return ["user-event"]
            return ["all-event"]

        output = diagnostics.build_discord_doctor_message(
            FakeDoctorBot(),
            222,
            empty_content_notice_count=3,
            get_mirrored_codex_thread_id_func=lambda _channel_id: "thread-1",
            get_mirror_project_for_channel_func=lambda _channel_id: ("project-key", "Project Name"),
            get_busy_choice_counts_func=lambda: (4, 5),
            get_persistent_component_claim_counts_func=lambda: (6, 7),
            build_mirror_check_func=build_mirror_check,
            get_discord_log_markers_func=lambda: markers,
            get_recent_discord_hook_events_func=recent_events,
            discord_qa_commands_enabled_func=lambda: True,
        )

        self.assertIn("Discord adapter diagnostics", output)
        self.assertIn("channel_id: 222", output)
        self.assertIn("mapped_thread_id: thread-1", output)
        self.assertIn("project_channel: Project Name", output)
        self.assertIn("history_poll_alive: True", output)
        self.assertIn("allowed_channels: 222,333", output)
        self.assertIn("Mapped thread controls:", output)
        self.assertIn("!stop", output)
        self.assertIn("Mirror check failed", output)
        self.assertIn("ERROR: mirror broke", output)
        self.assertIn("user-event", output)
        self.assertIn("all-event", output)


def _text_len(text: str | None) -> int:
    return len(text or "")


class FakeHistoryChannel:
    def __init__(
        self,
        messages: list[FakeHistoryMessage],
        *,
        error: BaseException | None = None,
    ) -> None:
        self._messages: list[FakeHistoryMessage] = messages
        self._error: BaseException | None = error

    def history(self, *, limit: int) -> AsyncIterator[FakeHistoryMessage]:
        async def iterator() -> AsyncIterator[FakeHistoryMessage]:
            if self._error is not None:
                raise self._error
            for message in self._messages[:limit]:
                yield message

        return iterator()


class FakeIdentifiedHistoryChannel(FakeHistoryChannel):
    id: int = 111


def _message(
    *,
    content: str,
    bot: bool,
    author_id: int = 242,
    type_name: str = "default",
) -> FakeHistoryMessage:
    return FakeHistoryMessage(
        created_at=datetime.datetime(
            2026,
            6,
            3,
            15,
            12,
            tzinfo=datetime.timezone.utc,
        ),
        author=FakeAuthor(id=author_id, bot=bot),
        content=content,
        type=FakeMessageType(name=type_name),
    )


class DiscordHistoryDiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_channel_history_lines_sanitize_message_content(self) -> None:
        lines = await diagnostics.build_discord_channel_history_lines(
            FakeIdentifiedHistoryChannel([
                _message(content="sensitive prompt", bot=False),
            ]),
            format_log_text_len_func=_text_len,
        )

        output = "\n".join(lines)

        self.assertIn("Recent channel history:", output)
        self.assertIn(
            "2026-06-03T15:12:00+00:00 bot=False content_len=16 type=default",
            output,
        )
        self.assertNotIn("sensitive prompt", output)

    async def test_channel_history_lines_report_unavailable_and_errors(self) -> None:
        self.assertEqual(
            await diagnostics.build_discord_channel_history_lines(
                None,
                format_log_text_len_func=_text_len,
            ),
            ["Recent channel history:", "history_unavailable: no_channel"],
        )
        self.assertEqual(
            await diagnostics.build_discord_channel_history_lines(
                FakeIdentifiedHistoryChannel([], error=ValueError("history failed")),
                format_log_text_len_func=_text_len,
            ),
            ["Recent channel history:", "history_error: ValueError"],
        )

    async def test_resolve_history_channel_reports_cache_fetch_and_errors(self) -> None:
        cached_channel = FakeHistoryChannel([])
        fetched_channel = FakeHistoryChannel([])

        class CachedBot:
            def get_cached_channel_or_thread(self, channel_id: int) -> tuple[FakeHistoryChannel, str]:
                if channel_id != 111:
                    raise AssertionError(channel_id)
                return cached_channel, "fake_cache"

        class FetchBot:
            async def fetch_channel(self, channel_id: int) -> FakeHistoryChannel:
                if channel_id != 222:
                    raise AssertionError(channel_id)
                return fetched_channel

        class FetchErrorBot:
            async def fetch_channel(self, channel_id: int) -> FakeHistoryChannel:
                if channel_id != 333:
                    raise AssertionError(channel_id)
                raise FetchChannelError("fetch failed")

        self.assertEqual(
            await diagnostics.resolve_discord_history_channel(CachedBot(), 111),
            (cached_channel, "fake_cache"),
        )
        self.assertEqual(
            await diagnostics.resolve_discord_history_channel(FetchBot(), 222),
            (fetched_channel, "fetch"),
        )
        self.assertEqual(
            await diagnostics.resolve_discord_history_channel(FetchErrorBot(), 333),
            (None, "fetch_error:FetchChannelError"),
        )

    async def test_tracked_target_history_selects_latest_non_bot_and_sanitizes(self) -> None:
        user_message = _message(content="sensitive prompt", bot=False, author_id=242)
        bot_message = _message(content="bot startup", bot=True, author_id=151)

        class FakeBot:
            allowed_channel_ids: set[int] = {222}
            startup_channel_id: int = 111

            def get_cached_channel_or_thread(
                self,
                channel_id: int,
            ) -> tuple[FakeHistoryChannel | None, str]:
                channels: dict[int, FakeHistoryChannel] = {
                    111: FakeHistoryChannel([bot_message]),
                    222: FakeHistoryChannel([bot_message, user_message]),
                }
                return channels.get(channel_id), "fake_cache"

        def get_targets(
            allowed_channel_ids: set[int],
            startup_channel_id: int | None,
            *,
            limit: int = 30,
        ) -> list[tuple[str, int]]:
            self.assertEqual(allowed_channel_ids, {222})
            self.assertEqual(startup_channel_id, 111)
            self.assertEqual(limit, 50)
            return [("startup", 111), ("allowed", 222)]

        output = "\n".join(
            await diagnostics.build_discord_tracked_target_user_history_lines(
                FakeBot(),
                get_startup_probe_targets_func=get_targets,
                format_log_text_len_func=_text_len,
            )
        )

        self.assertIn("Recent tracked target user history:", output)
        self.assertIn("startup channel=111 source=fake_cache latest_user=-", output)
        expected_allowed_line = (
            "allowed channel=222 source=fake_cache "
            + "latest_user_at=2026-06-03T15:12:00+00:00 "
            + "user=242 content_len=16 type=default"
        )
        self.assertIn(expected_allowed_line, output)
        self.assertNotIn("sensitive prompt", output)
