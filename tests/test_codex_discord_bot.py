from __future__ import annotations

import argparse
import asyncio
import io
import json
import os
import tempfile
import threading
import time
import unittest
from unittest import mock
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace
from typing import TYPE_CHECKING, Literal, final, overload, override

import discord
from aiohttp import ClientSession
from discord.state import ConnectionState
import codex_discord_bot as bot
import codex_discord_busy as discord_busy
import codex_discord_message_gate as message_gate
import codex_desktop_bridge as bridge
import codex_desktop_bridge_final_answer_types as final_answer_types
import codex_desktop_bridge_ipc_start_turn as ipc_start_turn
import codex_desktop_bridge_protocol as bridge_protocol
import codex_desktop_bridge_sidecar_resolver as sidecar_resolver
import codex_windows_harness as harness

if TYPE_CHECKING:
    from discord.types.channel import DMChannel as DiscordDMChannelPayload
    from discord.types.interactions import (
        ApplicationCommandInteraction as DiscordApplicationCommandInteraction,
    )
    from discord.types.user import User as DiscordUserPayload


def _require_json_object(
    value: ipc_start_turn.JsonValue | None,
) -> ipc_start_turn.JsonObject:
    if not isinstance(value, dict):
        raise AssertionError(f"Expected JSON object, got {value!r}")
    return value


def _final_turn_events(text: str) -> list[dict[str, object]]:
    return [
        {
            "timestamp": "1",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "phase": "final_answer",
                "content": [{"type": "output_text", "text": text}],
            },
        },
        {
            "timestamp": "2",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": "turn-1",
                "last_agent_message": text,
            },
        },
    ]


@final
class DiscordTestClient(discord.Client):
    @property
    def connection_state(self) -> ConnectionState:
        return self._connection


@final
class DiscordFakeFollowup(discord.Webhook):
    def __init__(
        self,
        interaction: discord.Interaction[discord.Client],
        state: ConnectionState,
        session: ClientSession,
    ) -> None:
        if not isinstance(interaction.channel, discord.DMChannel):
            raise AssertionError("Expected a DM channel for the test interaction")
        super().__init__(
            data={
                "id": str(interaction.application_id),
                "type": 3,
                "token": interaction.token,
            },
            session=session,
            token=interaction.token,
            state=state,
        )
        self.messages: list[object] = []
        self.kwargs: list[dict[str, object]] = []
        self.return_message: discord.WebhookMessage = discord.WebhookMessage(
            state=state,
            channel=interaction.channel,
            data={
                "id": "1001",
                "channel_id": str(interaction.channel.id),
                "author": {
                    "id": "1",
                    "username": "test-user",
                    "discriminator": "0",
                    "avatar": None,
                    "global_name": None,
                },
                "timestamp": "2026-01-01T00:00:00+00:00",
                "edited_timestamp": None,
                "tts": False,
                "mention_everyone": False,
                "mentions": [],
                "mention_roles": [],
                "attachments": [],
                "embeds": [],
                "pinned": False,
                "type": 0,
                "content": "",
            },
        )

    @overload
    async def send(
        self,
        *,
        view: discord.ui.LayoutView,
        wait: Literal[True],
        **kwargs: object,
    ) -> discord.WebhookMessage: ...

    @overload
    async def send(
        self,
        *,
        view: discord.ui.LayoutView,
        wait: Literal[False] = False,
        **kwargs: object,
    ) -> None: ...

    @overload
    async def send(
        self,
        content: object = "",
        *,
        view: discord.ui.View = ...,
        wait: Literal[True],
        **kwargs: object,
    ) -> discord.WebhookMessage: ...

    @overload
    async def send(
        self,
        content: object = "",
        *,
        view: discord.ui.View = ...,
        wait: Literal[False] = False,
        **kwargs: object,
    ) -> None: ...

    @override
    async def send(
        self,
        content: object = "",
        *,
        view: object = None,
        wait: bool = False,
        **kwargs: object,
    ) -> discord.WebhookMessage | None:
        self.messages.append(content if view is None else (content, view))
        self.kwargs.append(kwargs)
        return self.return_message if wait else None


class FakeFollowup:
    def __init__(self) -> None:
        self.messages: list[object] = []
        self.kwargs: list[dict[str, object]] = []

    async def send(self, content: str, view=None, **kwargs) -> None:
        self.messages.append(content if view is None else (content, view))
        self.kwargs.append(kwargs)


class FailingFollowup:
    def __init__(self, fail_after: int = 0) -> None:
        self.messages: list[object] = []
        self.fail_after = fail_after

    async def send(self, content: str, view=None, **kwargs) -> None:
        if len(self.messages) >= self.fail_after:
            raise RuntimeError("followup unavailable")
        self.messages.append(content if view is None else (content, view))


class NoViewKeywordFollowup:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.kwargs: list[dict[str, object]] = []

    async def send(self, content: str, **kwargs) -> None:
        if "view" in kwargs:
            raise TypeError("send() got an unexpected keyword argument 'view'")
        self.messages.append(content)
        self.kwargs.append(kwargs)


@final
class FakeInteractionCallbackResponse(
    discord.InteractionCallbackResponse[discord.Client]
):
    def __init__(
        self,
        parent: discord.Interaction[discord.Client],
        state: ConnectionState,
    ) -> None:
        super().__init__(
            data={
                "interaction": {
                    "id": str(parent.id),
                    "type": discord.InteractionType.application_command.value,
                }
            },
            parent=parent,
            state=state,
            type=discord.InteractionResponseType.channel_message,
        )


@final
class DiscordFakeResponse(discord.InteractionResponse[discord.Client]):
    def __init__(
        self,
        parent: discord.Interaction[discord.Client],
        state: ConnectionState,
    ) -> None:
        super().__init__(parent)
        self.parent: discord.Interaction[discord.Client] = parent
        self.state: ConnectionState = state
        self.messages: list[str] = []
        self.send_message_kwargs: list[dict[str, bool]] = []
        self.deferred: bool = False
        self.done: bool = False
        self.defer_kwargs: list[dict[str, object]] = []

    @override
    async def send_message(
        self,
        content: object | None = None,
        *,
        ephemeral: bool = False,
        **kwargs: object,
    ) -> discord.InteractionCallbackResponse[discord.Client]:
        _ = kwargs
        self.messages.append(str(content))
        self.send_message_kwargs.append({"ephemeral": ephemeral})
        self.done = True
        return FakeInteractionCallbackResponse(self.parent, self.state)

    @override
    async def defer(
        self,
        *,
        ephemeral: bool = False,
        thinking: bool = False,
    ) -> discord.InteractionCallbackResponse[discord.Client] | None:
        self.deferred = True
        self.done = True
        self.defer_kwargs.append({"thinking": thinking, "ephemeral": ephemeral})
        return None

    @override
    def is_done(self) -> bool:
        return self.done


class FakeResponse:
    def __init__(self) -> None:
        self.messages: list[str] = []
        self.send_message_kwargs: list[dict[str, bool]] = []
        self.deferred = False
        self.done = False
        self.defer_kwargs: list[dict[str, object]] = []

    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        self.messages.append(content)
        self.send_message_kwargs.append({"ephemeral": ephemeral})
        self.done = True

    async def defer(self, thinking: bool = False, **kwargs) -> None:
        self.deferred = True
        self.done = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})

    def is_done(self) -> bool:
        return self.done


class AlreadyAcknowledgedFakeResponse(FakeResponse):
    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        raise RuntimeError("Interaction has already been acknowledged.")


class AlreadyRespondedFakeResponse(FakeResponse):
    async def send_message(self, content: str, ephemeral: bool = False) -> None:
        raise RuntimeError("This interaction has already been responded to before.")


class FakeInteractionMessage:
    _next_id = 1000

    def __init__(self) -> None:
        self.id = FakeInteractionMessage._next_id
        FakeInteractionMessage._next_id += 1
        self.edits: list[object | None] = []
        self.components: list[object] = []

    async def edit(self, view=None) -> None:
        self.edits.append(view)
        if view is None:
            self.components = []


@final
class DiscordFailingEditInteractionMessage(discord.InteractionMessage):
    def __init__(
        self,
        state: ConnectionState,
        channel: discord.DMChannel,
        exc: BaseException,
    ) -> None:
        super().__init__(
            state=state,
            channel=channel,
            data={
                "id": "1000",
                "channel_id": str(channel.id),
                "author": {
                    "id": "1",
                    "username": "test-user",
                    "discriminator": "0",
                    "avatar": None,
                    "global_name": None,
                },
                "content": "",
                "timestamp": "2026-01-01T00:00:00+00:00",
                "edited_timestamp": None,
                "tts": False,
                "mention_everyone": False,
                "mentions": [],
                "mention_roles": [],
                "attachments": [],
                "embeds": [],
                "pinned": False,
                "type": 0,
                "components": [],
            },
        )
        self.exc: BaseException = exc

    @override
    async def edit(self, **kwargs: object) -> discord.InteractionMessage:
        _ = kwargs
        raise self.exc


class FailingEditInteractionMessage(FakeInteractionMessage):
    def __init__(self, exc: BaseException) -> None:
        super().__init__()
        self.exc = exc

    async def edit(self, view=None) -> None:
        _ = view
        raise self.exc


class FakeTyping:
    def __init__(self, target: "FakeTarget") -> None:
        self.target = target

    async def __aenter__(self) -> None:
        self.target.typing_events.append("enter")

    async def __aexit__(self, exc_type, exc, tb) -> None:
        self.target.typing_events.append("exit")


class FakeTarget:
    def __init__(self, channel_id: int = 222, parent_id: int | None = None) -> None:
        self.messages: list[tuple[str, object | None]] = []
        self.typing_events: list[str] = []
        self.id = channel_id
        self.parent_id = parent_id

    async def send(self, content: str, view=None) -> None:
        self.messages.append((content, view))

    def typing(self) -> FakeTyping:
        return FakeTyping(self)


class TransientFailingTarget(FakeTarget):
    def __init__(self, channel_id: int = 222, parent_id: int | None = None, *, failures: int = 1) -> None:
        super().__init__(channel_id=channel_id, parent_id=parent_id)
        self.failures = failures

    async def send(self, content: str, view=None) -> None:
        if self.failures > 0:
            self.failures -= 1
            raise RuntimeError("transient send failure")
        await super().send(content, view=view)


class AlwaysFailingTarget(FakeTarget):
    async def send(self, content: str, view=None) -> None:
        raise RuntimeError("send unavailable")


class ReturningTarget:
    def __init__(self, channel_id: int = 222, parent_id: int | None = None) -> None:
        self.messages: list[tuple[str, object | None]] = []
        self.id: int = channel_id
        self.parent_id: int | None = parent_id
        self.sent_messages: list[FakeInteractionMessage] = []

    async def send(self, content: str, view=None) -> FakeInteractionMessage:
        self.messages.append((content, view))
        sent_message = FakeInteractionMessage()
        if view is not None:
            sent_message.components = [
                SimpleNamespace(
                    children=[
                        SimpleNamespace(custom_id=getattr(item, "custom_id", None))
                        for item in getattr(view, "children", [])
                    ]
                )
            ]
        self.sent_messages.append(sent_message)
        return sent_message


class ViewFailingTarget(FakeTarget):
    async def send(self, content: str, view=None) -> None:
        if view is not None:
            raise RuntimeError("view rejected")
        await super().send(content, view=view)


@final
class DiscordFakeInteraction(discord.Interaction[discord.Client]):
    def __init__(
        self,
        command_name: str = "help",
        channel_id: int = 12345,
        *,
        user_id: int = 242286902982606848,
        message_failure: BaseException | None = None,
    ) -> None:
        _ = command_name
        self.fake_client: DiscordTestClient = DiscordTestClient(
            intents=discord.Intents.none()
        )
        state = self.fake_client.connection_state
        user_payload: DiscordUserPayload = {
            "id": str(user_id),
            "username": "test-user",
            "discriminator": "0",
            "avatar": None,
            "global_name": None,
        }
        interaction_payload: DiscordApplicationCommandInteraction = {
            "id": "1",
            "type": discord.InteractionType.application_command.value,
            "token": "test-token",
            "version": 1,
            "application_id": "1",
            "attachment_size_limit": 8_388_608,
            "authorizing_integration_owners": {},
            "channel": {
                "id": str(channel_id),
                "type": 1,
                "name": "test-dm",
                "last_message_id": None,
            },
            "data": {
                "id": "1",
                "name": command_name,
                "type": 1,
            },
        }
        super().__init__(
            data=interaction_payload,
            state=state,
        )
        self.user = discord.User(
            state=state,
            data=user_payload,
        )
        client_user = discord.ClientUser(
            state=state,
            data=user_payload,
        )
        channel_payload: DiscordDMChannelPayload = {
            "id": str(channel_id),
            "type": 1,
            "name": "test-dm",
            "last_message_id": None,
            "recipients": [],
        }
        self.fake_channel: discord.DMChannel = discord.DMChannel(
            me=client_user,
            state=state,
            data=channel_payload,
        )
        self.channel = self.fake_channel
        if message_failure is not None:
            self.message = DiscordFailingEditInteractionMessage(
                state,
                self.fake_channel,
                message_failure,
            )
        self.fake_followup: DiscordFakeFollowup = DiscordFakeFollowup(
            self,
            state,
            self._session,
        )
        self.fake_response: DiscordFakeResponse = DiscordFakeResponse(self, state)
        self._cs_followup = self.fake_followup
        self._cs_response = self.fake_response
        self._cs_command = None


class FakeInteraction:
    def __init__(self, command_name: str = "help", channel_id: int = 12345) -> None:
        self.command = SimpleNamespace(name=command_name)
        self.channel_id = channel_id
        self.followup = FakeFollowup()
        self.response = FakeResponse()
        self.user = SimpleNamespace(id=242286902982606848)
        self.channel = None
        self.message = FakeInteractionMessage()
        self.type = discord.InteractionType.application_command
        self.data: dict[str, object] = {}


class FakeAuthor:
    def __init__(
        self,
        author_id: int = 242286902982606848,
        *,
        bot: bool = False,
    ) -> None:
        self.id: int = author_id
        self.bot: bool = bot


class ReturningTargetMessage:
    def __init__(self, channel_id: int = 222) -> None:
        self.channel: ReturningTarget = ReturningTarget(channel_id=channel_id)
        self.author: FakeAuthor = FakeAuthor()


class FakeMessage:
    def __init__(self, content: str = "", channel_id: int = 222, message_id: int | None = None) -> None:
        self.id = message_id
        self.channel = FakeTarget(channel_id=channel_id)
        self.author: FakeAuthor = FakeAuthor()
        self.content = content
        self.raw_mentions: list[int] = []
        self.mentions: list[object] = []
        self.attachments: list[object] = []
        self.embeds: list[object] = []
        self.stickers: list[object] = []


class FakeAttachment:
    def __init__(
        self,
        filename: str,
        data: bytes,
        *,
        content_type: str = "application/octet-stream",
    ) -> None:
        self.filename = filename
        self._data = data
        self.content_type = content_type
        self.size = len(data)

    async def save(self, destination: Path) -> None:
        Path(destination).write_bytes(self._data)


class IdleSidecarClient:
    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
    ) -> ipc_start_turn.JsonObject:
        _ = thread_id, include_turns
        return {"thread": {"status": {"type": "idle"}}}

    def close(self) -> None:
        return None


class PreflightTargetBridge:
    def __init__(self, thread: bridge.ThreadInfo, busy_failure_message: str) -> None:
        self.thread: bridge.ThreadInfo = thread
        self.busy_failure_message: str = busy_failure_message

    def choose_thread(
        self,
        thread_id: str | None,
        cwd: str | None,
    ) -> bridge.ThreadInfo:
        _ = thread_id, cwd
        return self.thread

    def get_thread_workspace_ref(self, thread: bridge.ThreadInfo) -> str:
        _ = thread
        return "repo:1"

    def get_thread_label(self, thread: bridge.ThreadInfo) -> str:
        _ = thread
        return "repo:1"

    def get_thread_busy_state(
        self,
        thread: bridge.ThreadInfo,
        *,
        allow_resume: bool = True,
    ) -> str:
        _ = thread, allow_resume
        raise AssertionError(self.busy_failure_message)

    def get_busy_threads(self, limit: int = 50) -> list[bridge.ThreadInfo]:
        _ = limit
        raise AssertionError(self.busy_failure_message)


class DesktopDiscoveryStub:
    def __init__(
        self,
        result: tuple[Path | None, str] = (None, ""),
        failure: RuntimeError | None = None,
    ) -> None:
        self.result: tuple[Path | None, str] = result
        self.failure: RuntimeError | None = failure

    def discover_codex_desktop_executable(self) -> tuple[Path | None, str]:
        if self.failure is not None:
            raise self.failure
        return self.result


class ProjectCleanupGuildStub:
    def __init__(self, channel: object) -> None:
        self.channel: object = channel

    def get_channel(self, channel_id: int, /) -> object | None:
        return self.channel if channel_id == 111 else None

    async def fetch_channel(self, channel_id: int, /) -> object:
        _ = channel_id
        return self.channel


class ProjectCleanupCategoryStub:
    def __init__(self, category_id: int) -> None:
        self.id: int = category_id


class FakeBot:
    def __init__(self, *, allowed_user: bool = True, allowed_channel: bool = False) -> None:
        self.allowed_user = allowed_user
        self.allowed_channel = allowed_channel

    def is_allowed_user(self, user_id: int | None) -> bool:
        return self.allowed_user

    def is_allowed_channel(self, channel_id: int | None) -> bool:
        return self.allowed_channel

    def is_allowed_message_channel(self, channel) -> bool:
        return False


class EnvPatch:
    def __init__(self, key: str, value: str) -> None:
        self.key = key
        self.value = value
        self.original: str | None = None

    def __enter__(self) -> None:
        self.original = os.environ.get(self.key)
        os.environ[self.key] = self.value

    def __exit__(self, exc_type, exc, tb) -> None:
        if self.original is None:
            os.environ.pop(self.key, None)
        else:
            os.environ[self.key] = self.original


def _restore_env(name: str, previous: str | None) -> None:
    if previous is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = previous


def _restore_dict(target: dict[str, float], snapshot: dict[str, float]) -> None:
    target.clear()
    target.update(snapshot)


def _restore_set(target: set[str], snapshot: set[str]) -> None:
    target.clear()
    target.update(snapshot)


def _restore_mirror_db_path(previous: Path) -> None:
    bot.MIRROR_DB_PATH = previous


class DiscordBotHelperTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        mirror_db_temp_dir = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self.addCleanup(mirror_db_temp_dir.cleanup)

        old_app_server_transport_env = os.environ.get("CODEX_DISCORD_APP_SERVER_TRANSPORT")
        self.addCleanup(
            _restore_env,
            "CODEX_DISCORD_APP_SERVER_TRANSPORT",
            old_app_server_transport_env,
        )
        os.environ["CODEX_DISCORD_APP_SERVER_TRANSPORT"] = "0"

        old_mirror_db_path = bot.MIRROR_DB_PATH
        self.addCleanup(_restore_mirror_db_path, old_mirror_db_path)
        bot.MIRROR_DB_PATH = Path(mirror_db_temp_dir.name) / "mirror.sqlite"

        old_discord_log_path = os.environ.get("CODEX_DISCORD_LOG_PATH")
        self.addCleanup(_restore_env, "CODEX_DISCORD_LOG_PATH", old_discord_log_path)
        os.environ["CODEX_DISCORD_LOG_PATH"] = str(
            Path(mirror_db_temp_dir.name) / "test_discord_bot.log"
        )

        mirror_state = bot.get_session_mirror_state()
        old_active_output_targets = dict(mirror_state.active_output_targets)
        self.addCleanup(_restore_dict, mirror_state.active_output_targets, old_active_output_targets)
        mirror_state.active_output_targets.clear()

        old_pending_cursor_targets = set(mirror_state.pending_cursor_targets)
        self.addCleanup(_restore_set, mirror_state.pending_cursor_targets, old_pending_cursor_targets)
        mirror_state.pending_cursor_targets.clear()

        self.addCleanup(bot.ACTIVE_DISCORD_DELIVERIES.clear)
        bot.ACTIVE_DISCORD_DELIVERIES.clear()
        self.addCleanup(bot.clear_discord_delivery_stopping)
        bot.clear_discord_delivery_stopping()

        goal_status_patch = mock.patch.object(
            bot.app_server_transport.DEFAULT_CLIENT,
            "get_thread_goal_status",
            return_value=None,
        )
        _ = goal_status_patch.start()
        self.addCleanup(goal_status_patch.stop)
        bot.init_mirror_db()

    def _make_message_test_client(
        self,
        *,
        allowed_user_ids: set[int],
        plain_ask_mention_user_ids: set[int] | None = None,
    ) -> bot.CodexDiscordBot:
        client = bot.CodexDiscordBot(
            allowed_channel_ids=set(),
            allowed_user_ids=allowed_user_ids,
            startup_channel_id=None,
            guild_id=None,
            enable_prefix_commands=True,
            plain_ask_mention_user_ids=plain_ask_mention_user_ids,
        )
        self.addAsyncCleanup(client.close)
        return client

    def test_bridge_stream_runs_in_subprocess_without_cross_thread_output(self) -> None:
        original_get_bridge_script_path = bot.get_bridge_script_path
        outputs: dict[str, tuple[int, str, list[str]]] = {}

        with tempfile.TemporaryDirectory() as temp_dir:
            script_path = Path(temp_dir) / "bridge_stream_fixture.py"
            script_path.write_text(
                "\n".join(
                    [
                        "from __future__ import annotations",
                        "import sys",
                        "import time",
                        "name = sys.argv[1]",
                        "print(f'{name}:start', flush=True)",
                        "time.sleep(0.05)",
                        "print(f'{name}:end', flush=True)",
                    ]
                ),
                encoding="utf-8",
            )

            def worker(name: str) -> None:
                lines: list[str] = []
                exit_code, output = bot.run_bridge_command_stream([name], lines.append)
                outputs[name] = (exit_code, output, lines)

            try:
                bot.get_bridge_script_path = lambda: script_path
                threads = [
                    threading.Thread(target=worker, args=("a",)),
                    threading.Thread(target=worker, args=("b",)),
                ]
                for thread in threads:
                    thread.start()
                for thread in threads:
                    thread.join()
            finally:
                bot.get_bridge_script_path = original_get_bridge_script_path

        self.assertEqual(outputs["a"], (0, "a:start\na:end", ["a:start", "a:end"]))
        self.assertEqual(outputs["b"], (0, "b:start\nb:end", ["b:start", "b:end"]))

    def test_bridge_stream_forces_utf8_child_output(self) -> None:
        original_get_bridge_script_path = bot.get_bridge_script_path
        with tempfile.TemporaryDirectory() as temp_dir:
            script_path = Path(temp_dir) / "bridge_utf8_child.py"
            script_path.write_text(
                "\n".join(
                    [
                        "from __future__ import annotations",
                        "import os",
                        "print(os.environ.get('PYTHONIOENCODING', '-'))",
                        "print(os.environ.get('PYTHONUTF8', '-'))",
                        "print(os.environ.get('PYTHONUNBUFFERED', '-'))",
                        "print('\\ud55c\\uae00 approval\\ud14c\\uc2a4\\ud2b8')",
                    ]
                ),
                encoding="utf-8",
            )
            lines: list[str] = []
            try:
                bot.get_bridge_script_path = lambda: script_path
                exit_code, output = bot.run_bridge_command_stream([], lines.append)
            finally:
                bot.get_bridge_script_path = original_get_bridge_script_path

        self.assertEqual(exit_code, 0)
        self.assertEqual(lines, ["utf-8", "1", "1", "\ud55c\uae00 approval\ud14c\uc2a4\ud2b8"])
        self.assertEqual(output, "\n".join(lines))

    def test_bridge_stream_delivers_child_lines_before_process_exit(self) -> None:
        original_get_bridge_script_path = bot.get_bridge_script_path
        start_seen = threading.Event()
        finished = threading.Event()
        lines: list[str] = []
        result: dict[str, object] = {}

        with tempfile.TemporaryDirectory() as temp_dir:
            script_path = Path(temp_dir) / "bridge_unbuffered_child.py"
            script_path.write_text(
                "\n".join(
                    [
                        "from __future__ import annotations",
                        "import time",
                        "print('child:start')",
                        "time.sleep(2.0)",
                        "print('child:end')",
                    ]
                ),
                encoding="utf-8",
            )

            def on_line(line: str) -> None:
                lines.append(line)
                if line == "child:start":
                    start_seen.set()

            def worker() -> None:
                try:
                    exit_code, output = bot.run_bridge_command_stream([], on_line)
                    result["exit_code"] = exit_code
                    result["output"] = output
                finally:
                    finished.set()

            try:
                bot.get_bridge_script_path = lambda: script_path
                thread = threading.Thread(target=worker)
                thread.start()
                try:
                    self.assertTrue(start_seen.wait(timeout=1.0))
                    self.assertFalse(finished.is_set())
                finally:
                    thread.join(timeout=4.0)
            finally:
                bot.get_bridge_script_path = original_get_bridge_script_path

        self.assertFalse(thread.is_alive())
        self.assertEqual(result.get("exit_code"), 0)
        self.assertEqual(lines, ["child:start", "child:end"])
        self.assertEqual(result.get("output"), "child:start\nchild:end")

    def test_run_ask_stream_uses_resident_app_server_for_explicit_target_thread(self) -> None:
        original_steer_or_start = bot.app_server_transport.steer_or_start_no_wait
        original_run_watch = bot.run_steering_watch_stream
        calls: list[tuple[str, str | None, object]] = []

        class FakeRelay:
            def __init__(self) -> None:
                self.lines: list[str] = []
                self.finished = False

            def feed_line(self, line: str) -> None:
                self.lines.append(line)

            def finish(self) -> None:
                self.finished = True

        def fake_steer_or_start(client, prompt, target_thread_id, **kwargs):
            calls.append((prompt, target_thread_id, client))
            return bot.app_server_transport.AppServerDeliveryResult(
                0,
                "transport: resident-app-server turn/start",
                thread_id="thread-1",
                turn_id="turn-1",
                target_ref="taxlab:1",
                session_path="session.jsonl",
                start_offset=10,
            )

        def fake_run_watch(steering_result, relay, *, timeout_sec=0):
            relay.finish()
            return 0, "watched"

        try:
            bot.app_server_transport.steer_or_start_no_wait = fake_steer_or_start
            bot.run_steering_watch_stream = fake_run_watch
            relay = FakeRelay()
            with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                exit_code, output = bot.run_ask_stream(
                    "please run",
                    relay,
                    target_thread_id="thread-1",
                )
        finally:
            bot.app_server_transport.steer_or_start_no_wait = original_steer_or_start
            bot.run_steering_watch_stream = original_run_watch

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "watched")
        self.assertEqual(calls, [("please run", "thread-1", bot.app_server_transport.DEFAULT_CLIENT)])
        self.assertTrue(relay.finished)

    def test_legacy_run_ask_stream_does_not_ui_fallback_after_ipc_failure(self) -> None:
        original_run_bridge_command_stream = bot.run_bridge_command_stream
        captured_argvs: list[list[str]] = []

        class FakeRelay:
            def __init__(self) -> None:
                self.lines: list[str] = []
                self.finished = False

            def feed_line(self, line: str) -> None:
                self.lines.append(line)

            def finish(self) -> None:
                self.finished = True

        def fake_run_bridge_command_stream(argv, on_line):
            captured_argvs.append(list(argv))
            return 1, "ERROR: IPC owner client for the selected thread was not discovered."

        try:
            bot.run_bridge_command_stream = fake_run_bridge_command_stream
            relay = FakeRelay()
            with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "0"):
                exit_code, output = bot.run_ask_stream(
                    "please run",
                    relay,
                    target_thread_id="thread-1",
                )
        finally:
            bot.run_bridge_command_stream = original_run_bridge_command_stream

        self.assertEqual(exit_code, 1)
        self.assertIn("IPC owner client", output)
        self.assertEqual(len(captured_argvs), 1)
        self.assertIn("--ipc", captured_argvs[0])
        self.assertIn("--no-fallback", captured_argvs[0])
        self.assertNotIn("--sidecar", captured_argvs[0])
        self.assertNotIn("--ui", captured_argvs[0])
        self.assertTrue(relay.finished)

    def test_run_transport_prompt_no_wait_uses_resident_app_server_transport_by_default(self) -> None:
        original_run_app_server = bot.discord_app_server.run_prompt_no_wait
        calls: list[tuple[str, str | None]] = []

        def fake_run_app_server(prompt, target_thread_id, **kwargs):
            calls.append((prompt, target_thread_id))
            return 0, "transport: resident-app-server turn/start"

        try:
            bot.discord_app_server.run_prompt_no_wait = fake_run_app_server
            with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                exit_code, output = bot.run_transport_prompt_no_wait("please run", "thread-1")
        finally:
            bot.discord_app_server.run_prompt_no_wait = original_run_app_server

        self.assertEqual(exit_code, 0)
        self.assertIn("resident-app-server", output)
        self.assertEqual(calls, [("please run", "thread-1")])

    def test_run_ask_uses_ipc_no_fallback_without_ui_retry(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        captured_argvs: list[list[str]] = []

        def fake_run_bridge_command(argv):
            captured_argvs.append(list(argv))
            return 1, "ERROR: IPC owner client for the selected thread was not discovered."

        try:
            bot.run_bridge_command = fake_run_bridge_command
            exit_code, output = bot.run_ask("please run", target_thread_id="thread-1")
        finally:
            bot.run_bridge_command = original_run_bridge_command

        self.assertEqual(exit_code, 1)
        self.assertIn("IPC owner client", output)
        self.assertEqual(len(captured_argvs), 1)
        self.assertIn("--ipc", captured_argvs[0])
        self.assertIn("--no-fallback", captured_argvs[0])
        self.assertNotIn("--ipc-recover-ui", captured_argvs[0])
        self.assertNotIn("--ui", captured_argvs[0])

    def test_run_transport_prompt_no_wait_surfaces_transport_failure_without_legacy_fallback(self) -> None:
        original_run_app_server = bot.discord_app_server.run_prompt_no_wait
        original_legacy = bot.run_legacy_ipc_prompt_no_wait

        def fake_run_app_server(prompt, target_thread_id, **kwargs):
            raise RuntimeError("transport boom")

        def fake_legacy(prompt, target_thread_id):
            raise AssertionError("transport failure must not use legacy IPC fallback")

        try:
            bot.discord_app_server.run_prompt_no_wait = fake_run_app_server
            bot.run_legacy_ipc_prompt_no_wait = fake_legacy
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                        with EnvPatch("CODEX_DISCORD_APP_SERVER_LEGACY_FALLBACK", "1"):
                            exit_code, output = bot.run_transport_prompt_no_wait(
                                "please run",
                                "thread-1",
                            )
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.discord_app_server.run_prompt_no_wait = original_run_app_server
            bot.run_legacy_ipc_prompt_no_wait = original_legacy

        self.assertEqual(exit_code, 1)
        self.assertEqual(output, "ERROR: resident app-server transport failed: transport boom")
        self.assertIn("app_server_prompt_failed target=thread-1", log_text)
        self.assertIn("error_type=RuntimeError error=transport boom", log_text)
        self.assertNotIn("fallback=", log_text)

    def test_run_app_server_steering_surfaces_transport_failure_without_legacy_fallback(self) -> None:
        original_run_steering = bot.discord_app_server.run_steering_no_wait
        original_legacy = bot.discord_steering.run_steering_prompt
        original_resolve_target_ref = bot.resolve_target_ref

        def fake_run_steering(*args, **kwargs):
            raise RuntimeError("steer transport boom")

        def fake_legacy(*args, **kwargs):
            raise AssertionError("steering transport failure must not use legacy fallback")

        try:
            bot.discord_app_server.run_steering_no_wait = fake_run_steering
            bot.discord_steering.run_steering_prompt = fake_legacy
            bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "taxlab:1")
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_LEGACY_FALLBACK", "1"):
                        result = bot.run_resident_app_server_steering_prompt("please run", "thread-1")
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.discord_app_server.run_steering_no_wait = original_run_steering
            bot.discord_steering.run_steering_prompt = original_legacy
            bot.resolve_target_ref = original_resolve_target_ref

        self.assertEqual(result.exit_code, 1)
        self.assertEqual(
            result.output,
            "ERROR: resident app-server transport failed: steer transport boom",
        )
        self.assertEqual(result.target_thread_id, "thread-1")
        self.assertEqual(result.target_ref, "taxlab:1")
        self.assertIn("app_server_steering_failed target=thread-1", log_text)
        self.assertIn("error_type=RuntimeError error=steer transport boom", log_text)
        self.assertNotIn("fallback=", log_text)

    def test_run_ask_stream_surfaces_transport_failure_without_stream_fallback(self) -> None:
        original_steer_or_start = bot.app_server_transport.steer_or_start_no_wait
        original_stream = bot.discord_stream.run_ask_stream

        class FakeRelay:
            def __init__(self) -> None:
                self.finished = False

            def feed_line(self, line: str) -> None:
                raise AssertionError("transport failure should not stream bridge output")

            def finish(self) -> None:
                self.finished = True

        def fake_steer_or_start(*args, **kwargs):
            raise RuntimeError("stream transport boom")

        def fake_stream(*args, **kwargs):
            raise AssertionError("transport failure must not use stream fallback")

        try:
            bot.app_server_transport.steer_or_start_no_wait = fake_steer_or_start
            bot.discord_stream.run_ask_stream = fake_stream
            relay = FakeRelay()
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    with EnvPatch("CODEX_DISCORD_APP_SERVER_TRANSPORT", "1"):
                        with EnvPatch("CODEX_DISCORD_APP_SERVER_LEGACY_FALLBACK", "1"):
                            exit_code, output = bot.run_ask_stream(
                                "please run",
                                relay,
                                target_thread_id="thread-1",
                            )
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.app_server_transport.steer_or_start_no_wait = original_steer_or_start
            bot.discord_stream.run_ask_stream = original_stream

        self.assertEqual(exit_code, 1)
        self.assertEqual(output, "ERROR: resident app-server transport failed: stream transport boom")
        self.assertTrue(relay.finished)
        self.assertIn("app_server_stream_prompt_failed target=thread-1", log_text)
        self.assertIn("error_type=RuntimeError error=stream transport boom", log_text)
        self.assertNotIn("fallback=", log_text)

    def test_running_app_server_executable_ignores_windowsapps_alias(self) -> None:
        original_run_powershell_capture = sidecar_resolver.run_powershell_capture
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                alias_path = Path(temp_dir) / "Microsoft" / "WindowsApps" / "codex.exe"
                alias_path.parent.mkdir(parents=True)
                alias_path.write_text("", encoding="utf-8")

                def fake_run_powershell_capture(command: str) -> str:
                    return str(alias_path)

                sidecar_resolver.run_powershell_capture = fake_run_powershell_capture

                path, source = bridge.detect_running_codex_app_server_executable()

            self.assertIsNone(path)
            self.assertEqual(source, "")
        finally:
            sidecar_resolver.run_powershell_capture = original_run_powershell_capture

    def test_legacy_ipc_prompt_no_wait_has_no_ui_recovery_or_transport_fallback(self) -> None:
        original_run_bridge_command = bot.run_bridge_command
        captured_argv: list[str] = []

        def fake_run_bridge_command(argv):
            captured_argv.extend(argv)
            return 0, "ok"

        try:
            bot.run_bridge_command = fake_run_bridge_command
            exit_code, output = bot.run_legacy_ipc_prompt_no_wait("please run", "thread-1")
        finally:
            bot.run_bridge_command = original_run_bridge_command

        self.assertEqual(exit_code, 0)
        self.assertEqual(output, "ok")
        self.assertIn("--ipc", captured_argv)
        self.assertNotIn("--ipc-recover-ui", captured_argv)
        self.assertIn("--foreground", captured_argv)
        self.assertIn("--no-fallback", captured_argv)
        self.assertIn("--no-wait", captured_argv)
        self.assertNotIn("--sidecar", captured_argv)
        self.assertNotIn("--ui", captured_argv)

    def test_command_ask_sidecar_transport_starts_turn_without_ipc(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_state = bridge.get_thread_busy_state
        original_start_sidecar = bridge.start_turn_via_sidecar
        original_start_ipc = bridge.start_turn_via_ipc
        original_wait_delivery = bridge.wait_for_prompt_delivery
        original_watch_final = bridge.watch_for_final_answer
        original_sync = bridge.sync_session_index_with_state

        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            session_path.write_text("", encoding="utf-8")
            thread_info = bridge.ThreadInfo(
                id="thread-1",
                title="Thread",
                cwd=temp_dir,
                updated_at=1,
                rollout_path=str(session_path),
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )
            calls: list[tuple[str, str]] = []

            def fake_start_sidecar(selected_thread, prompt, *, timeout_sec=10.0, keep_client_open=False):
                calls.append((selected_thread.id, prompt))
                return {"owner_client_id": "", "turn_id": "turn-1", "attempts": "1"}

            def fake_start_ipc(*args, **kwargs):
                raise AssertionError("sidecar ask must not call IPC start-turn")

            def fake_watch_final(**kwargs):
                return {
                    "commentary": [],
                    "final_answer": "done",
                    "final_streamed_live": False,
                    "status": "ready",
                }

            try:
                bridge.choose_thread = lambda thread_id=None, cwd=None: thread_info
                bridge.get_thread_busy_state = lambda selected_thread, **kwargs: "idle"
                bridge.start_turn_via_sidecar = fake_start_sidecar
                bridge.start_turn_via_ipc = fake_start_ipc
                bridge.wait_for_prompt_delivery = (
                    lambda recent_offsets, prompt, timeout_sec=6.0: thread_info
                )
                bridge.watch_for_final_answer = fake_watch_final
                bridge.sync_session_index_with_state = lambda: None

                parser = bridge.build_parser()
                args = parser.parse_args(
                    ["ask", "--sidecar", "--thread-id", "thread-1", "please run"]
                )
                stdout = io.StringIO()
                with redirect_stdout(stdout):
                    exit_code = args.func(args)
            finally:
                bridge.choose_thread = original_choose_thread
                bridge.get_thread_busy_state = original_get_busy_state
                bridge.start_turn_via_sidecar = original_start_sidecar
                bridge.start_turn_via_ipc = original_start_ipc
                bridge.wait_for_prompt_delivery = original_wait_delivery
                bridge.watch_for_final_answer = original_watch_final
                bridge.sync_session_index_with_state = original_sync

        text = stdout.getvalue()
        self.assertEqual(exit_code, 0)
        self.assertEqual(calls, [("thread-1", "please run")])
        self.assertIn("transport: local-sidecar turn/start", text)
        self.assertIn("[sidecar_delivery] turn_id=turn-1 attempts=1", text)
        self.assertIn("[final_answer]\ndone", text)

    def test_command_ask_default_does_not_use_sidecar_after_ipc_owner_failure(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_state = bridge.get_thread_busy_state
        original_start_sidecar = bridge.start_turn_via_sidecar
        original_start_ipc = bridge.start_turn_via_ipc
        original_sync = bridge.sync_session_index_with_state

        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            session_path.write_text("", encoding="utf-8")
            thread_info = bridge.ThreadInfo(
                id="thread-1",
                title="Thread",
                cwd=temp_dir,
                updated_at=1,
                rollout_path=str(session_path),
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )

            def fake_start_ipc(*args, **kwargs):
                raise RuntimeError("IPC owner client for the selected thread was not discovered in background mode.")

            def fake_start_sidecar(*args, **kwargs):
                raise AssertionError("no-fallback ask must not call sidecar")

            try:
                bridge.choose_thread = lambda thread_id=None, cwd=None: thread_info
                bridge.get_thread_busy_state = lambda thread, allow_resume=True: "idle"
                bridge.start_turn_via_ipc = fake_start_ipc
                bridge.start_turn_via_sidecar = fake_start_sidecar
                bridge.sync_session_index_with_state = lambda: None

                parser = bridge.build_parser()
                args = parser.parse_args(
                    ["ask", "--ipc", "--no-wait", "--thread-id", "thread-1", "please run"]
                )
                self.assertTrue(args.no_fallback)
                with self.assertRaises(RuntimeError) as raised:
                    args.func(args)
            finally:
                bridge.choose_thread = original_choose_thread
                bridge.get_thread_busy_state = original_get_busy_state
                bridge.start_turn_via_sidecar = original_start_sidecar
                bridge.start_turn_via_ipc = original_start_ipc
                bridge.sync_session_index_with_state = original_sync

        self.assertIn("IPC owner client", str(raised.exception))

    def test_wait_for_thread_activation_retries_until_header_matches(self) -> None:
        original_verify_header = bridge.verify_active_thread_by_header
        original_verify_thread = bridge.verify_active_thread
        original_sleep = bridge.time.sleep
        original_time = bridge.time.time
        now = [0.0]
        header_calls = []

        def fake_verify_header(thread_name: str) -> str | None:
            header_calls.append(thread_name)
            if len(header_calls) >= 3:
                return "header"
            return None

        thread_info = bridge.ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="C:\\repo\\session.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

        try:
            bridge.verify_active_thread_by_header = fake_verify_header
            bridge.verify_active_thread = lambda thread_id: None
            bridge.time.time = lambda: now[0]
            bridge.time.sleep = lambda seconds: now.__setitem__(0, now[0] + float(seconds))

            self.assertEqual(
                bridge.wait_for_thread_activation(thread_info, "Thread", timeout_sec=2.0),
                "header",
            )
        finally:
            bridge.verify_active_thread_by_header = original_verify_header
            bridge.verify_active_thread = original_verify_thread
            bridge.time.sleep = original_sleep
            bridge.time.time = original_time

        self.assertEqual(header_calls, ["Thread", "Thread", "Thread"])

    def test_activate_thread_in_ui_uses_wait_after_sidebar_click(self) -> None:
        original_name_candidates = bridge.get_thread_ui_name_candidates
        original_verify_header = bridge.verify_active_thread_by_header
        original_verify_thread = bridge.verify_active_thread
        original_activate_deep_link = bridge_protocol.open_codex_thread_deep_link
        original_activate_sidebar = bridge.activate_thread_by_sidebar_v2
        original_wait_activation = bridge.wait_for_thread_activation
        wait_calls = []

        thread_info = bridge.ThreadInfo(
            id="thread-1",
            title="Thread",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="C:\\repo\\session.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=1,
        )

        try:
            bridge.get_thread_ui_name_candidates = lambda thread: ["Thread"]
            bridge.verify_active_thread_by_header = lambda thread_name: None
            bridge.verify_active_thread = lambda thread_id: None

            def fail_deep_link(_thread_id: str) -> None:
                raise OSError("deep link unavailable")

            bridge_protocol.open_codex_thread_deep_link = fail_deep_link
            bridge.activate_thread_by_sidebar_v2 = lambda thread_name, project_name=None: "Thread row"

            def fake_wait_activation(thread: bridge.ThreadInfo, thread_name: str, timeout_sec: float = 5.0) -> str | None:
                wait_calls.append((thread.id, thread_name, timeout_sec))
                return "copy-session-id"

            bridge.wait_for_thread_activation = fake_wait_activation

            self.assertEqual(
                bridge.activate_thread_in_ui(thread_info),
                "sidebar:Thread row [copy-session-id]",
            )
        finally:
            bridge.get_thread_ui_name_candidates = original_name_candidates
            bridge.verify_active_thread_by_header = original_verify_header
            bridge.verify_active_thread = original_verify_thread
            bridge_protocol.open_codex_thread_deep_link = original_activate_deep_link
            bridge.activate_thread_by_sidebar_v2 = original_activate_sidebar
            bridge.wait_for_thread_activation = original_wait_activation

        self.assertEqual(wait_calls, [("thread-1", "Thread", 5.0)])

    def test_discord_busy_detection_ignores_other_thread_busy_error(self) -> None:
        output = "ERROR: Another mapped thread is still working."

        self.assertFalse(discord_busy.is_selected_thread_busy_error(1, output))
        self.assertFalse(discord_busy.is_selected_thread_busy_error(0, output))

    def test_discord_busy_detection_treats_ipc_timeout_as_thread_retryable(self) -> None:
        output = (
            "target_thread: 019e90d5-bd7a-7781-9979-e886f63781a7\n"
            "ui_activation: ipc-thread-follower-start-turn\n"
            "ERROR: Timed out waiting for IPC data from \\\\.\\pipe\\codex-ipc."
        )

        self.assertTrue(discord_busy.is_selected_thread_busy_error(1, output))

    def test_bridge_ipc_start_turn_inherits_summary_setting(self) -> None:
        original_write_ipc_message = bridge._write_ipc_message
        original_read_ipc_response = bridge._read_ipc_response
        written_payloads: list[ipc_start_turn.JsonObject] = []

        def write_ipc_message(
            _handle: int,
            payload: ipc_start_turn.JsonObject,
        ) -> None:
            written_payloads.append(payload)

        try:
            bridge._write_ipc_message = write_ipc_message
            bridge._read_ipc_response = lambda _handle, _request_id, timeout_sec, owner_clients: {
                "resultType": "success",
                "handledByClientId": "client-1",
                "result": {"result": {"turn": {"id": "turn-1"}}},
            }
            thread = bridge.ThreadInfo(
                id="thread-1",
                title="Thread",
                cwd="C:\\repo",
                updated_at=1,
                rollout_path="session.jsonl",
                model="gpt",
                reasoning_effort="high",
                tokens_used=0,
            )

            result = bridge._request_start_turn_via_ipc(
                1,
                "source-client",
                thread,
                "prompt",
                1.0,
                {},
            )

            self.assertEqual(result["turn_id"], "turn-1")
            params = _require_json_object(written_payloads[0].get("params"))
            turn_start_params = _require_json_object(params.get("turnStartParams"))
            self.assertTrue(turn_start_params["inheritThreadSettings"])
            self.assertIsNone(turn_start_params["summary"])
            self.assertIsNone(turn_start_params["serviceTier"])
        finally:
            bridge._write_ipc_message = original_write_ipc_message
            bridge._read_ipc_response = original_read_ipc_response

    def test_bridge_ipc_ask_ignores_other_busy_threads(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_threads = bridge.get_busy_threads
        original_get_thread_ui_name = bridge.get_thread_ui_name
        original_get_thread_busy_state = bridge.get_thread_busy_state
        original_start_turn_via_ipc = bridge.start_turn_via_ipc
        original_wait_for_prompt_delivery = bridge.wait_for_prompt_delivery
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("old", encoding="utf-8")
                target_thread = bridge.ThreadInfo(
                    id="target-thread",
                    title="Target",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=0,
                )
                busy_thread = bridge.ThreadInfo(
                    id="busy-thread",
                    title="Busy",
                    cwd=str(temp_dir),
                    updated_at=2,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=1,
                )

                bridge.choose_thread = lambda thread_id=None, cwd=None: target_thread
                bridge.get_busy_threads = lambda limit=50: [busy_thread]
                bridge.get_thread_ui_name = lambda thread_id, thread=None: None
                bridge.get_thread_busy_state = lambda thread, allow_resume=True: "idle"
                bridge.start_turn_via_ipc = lambda thread, prompt, timeout_sec=10.0, allow_ui_recovery=False: {
                    "owner_client_id": "client-1",
                    "turn_id": "turn-1",
                }
                bridge.wait_for_prompt_delivery = lambda session_offsets, prompt, timeout_sec=4.0: target_thread
                bridge.watch_for_final_answer = lambda **kwargs: {
                    "commentary": [],
                    "final_answer": "done",
                    "status": "ready",
                    "streamed_live": False,
                    "final_streamed_live": False,
                }
                args = argparse.Namespace(
                    thread_id="target-thread",
                    cwd=None,
                    prompt="qa prompt",
                    dry_run=False,
                    force_while_busy=False,
                    ipc=True,
                    ipc_recover_ui=False,
                    background=False,
                    wait=True,
                    timeout=30.0,
                    include_commentary=False,
                    stream=False,
                )

                stdout = io.StringIO()
                with redirect_stdout(stdout):
                    exit_code = bridge.command_ask(args)

            self.assertEqual(exit_code, 0)
            self.assertIn("[final_answer]\ndone", stdout.getvalue())
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_busy_threads = original_get_busy_threads
            bridge.get_thread_ui_name = original_get_thread_ui_name
            bridge.get_thread_busy_state = original_get_thread_busy_state
            bridge.start_turn_via_ipc = original_start_turn_via_ipc
            bridge.wait_for_prompt_delivery = original_wait_for_prompt_delivery
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_ipc_ask_keeps_selected_thread_busy_steerable(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_threads = bridge.get_busy_threads
        original_get_thread_ui_name = bridge.get_thread_ui_name
        original_is_thread_busy = bridge.is_thread_busy
        original_get_thread_busy_state = bridge.get_thread_busy_state
        original_start_turn_via_ipc = bridge.start_turn_via_ipc
        original_wait_for_prompt_delivery = bridge.wait_for_prompt_delivery
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("old", encoding="utf-8")
                target_thread = bridge.ThreadInfo(
                    id="target-thread",
                    title="Target",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=0,
                )

                bridge.choose_thread = lambda thread_id=None, cwd=None: target_thread
                bridge.get_busy_threads = lambda limit=50: [target_thread]
                bridge.get_thread_ui_name = lambda thread_id, thread=None: None
                bridge.is_thread_busy = lambda path: True
                bridge.get_thread_busy_state = lambda thread, allow_resume=True: "busy"
                start_calls: list[tuple[str, str]] = []
                bridge.start_turn_via_ipc = lambda thread, prompt, timeout_sec=10.0, allow_ui_recovery=False: (
                    start_calls.append((thread.id, prompt))
                    or {
                        "owner_client_id": "client-1",
                        "turn_id": "turn-1",
                    }
                )
                bridge.wait_for_prompt_delivery = lambda session_offsets, prompt, timeout_sec=4.0: target_thread
                bridge.watch_for_final_answer = lambda **kwargs: {
                    "commentary": [],
                    "final_answer": "done",
                    "status": "ready",
                    "streamed_live": False,
                    "final_streamed_live": False,
                }
                args = argparse.Namespace(
                    thread_id="target-thread",
                    cwd=None,
                    prompt="qa prompt",
                    dry_run=False,
                    force_while_busy=False,
                    ipc=True,
                    ipc_recover_ui=False,
                    background=False,
                    wait=True,
                    timeout=30.0,
                    include_commentary=False,
                    stream=False,
                )

                stdout = io.StringIO()
                with redirect_stdout(stdout):
                    exit_code = bridge.command_ask(args)

            self.assertEqual(exit_code, 0)
            self.assertEqual(start_calls, [("target-thread", "qa prompt")])
            self.assertIn("[ipc_delivery] owner_client=client-1 turn_id=turn-1", stdout.getvalue())
            self.assertIn("[final_answer]\ndone", stdout.getvalue())
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_busy_threads = original_get_busy_threads
            bridge.get_thread_ui_name = original_get_thread_ui_name
            bridge.is_thread_busy = original_is_thread_busy
            bridge.get_thread_busy_state = original_get_thread_busy_state
            bridge.start_turn_via_ipc = original_start_turn_via_ipc
            bridge.wait_for_prompt_delivery = original_wait_for_prompt_delivery
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_ipc_ask_keeps_busy_target_steerable_when_other_thread_busy(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_threads = bridge.get_busy_threads
        original_get_thread_ui_name = bridge.get_thread_ui_name
        original_is_thread_busy = bridge.is_thread_busy
        original_get_thread_busy_state = bridge.get_thread_busy_state
        original_start_turn_via_ipc = bridge.start_turn_via_ipc
        original_wait_for_prompt_delivery = bridge.wait_for_prompt_delivery
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("old", encoding="utf-8")
                target_thread = bridge.ThreadInfo(
                    id="target-thread",
                    title="Target",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=0,
                )
                other_thread = bridge.ThreadInfo(
                    id="other-thread",
                    title="Other",
                    cwd=str(temp_dir),
                    updated_at=2,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=1,
                )

                bridge.choose_thread = lambda thread_id=None, cwd=None: target_thread
                bridge.get_busy_threads = lambda limit=50: [other_thread]
                bridge.get_thread_ui_name = lambda thread_id, thread=None: None
                bridge.is_thread_busy = lambda path: True
                bridge.get_thread_busy_state = lambda thread, allow_resume=True: "busy"
                start_calls: list[tuple[str, str]] = []
                bridge.start_turn_via_ipc = lambda thread, prompt, timeout_sec=10.0, allow_ui_recovery=False: (
                    start_calls.append((thread.id, prompt))
                    or {
                        "owner_client_id": "client-1",
                        "turn_id": "turn-1",
                    }
                )
                bridge.wait_for_prompt_delivery = lambda session_offsets, prompt, timeout_sec=4.0: target_thread
                bridge.watch_for_final_answer = lambda **kwargs: {
                    "commentary": [],
                    "final_answer": "done",
                    "status": "ready",
                    "streamed_live": False,
                    "final_streamed_live": False,
                }
                args = argparse.Namespace(
                    thread_id="target-thread",
                    cwd=None,
                    prompt="qa prompt",
                    dry_run=False,
                    force_while_busy=False,
                    ipc=True,
                    ipc_recover_ui=False,
                    background=False,
                    wait=True,
                    timeout=30.0,
                    include_commentary=False,
                    stream=False,
                )

                stdout = io.StringIO()
                with redirect_stdout(stdout):
                    exit_code = bridge.command_ask(args)

            self.assertEqual(exit_code, 0)
            self.assertEqual(start_calls, [("target-thread", "qa prompt")])
            self.assertIn("[ipc_delivery] owner_client=client-1 turn_id=turn-1", stdout.getvalue())
            self.assertIn("[final_answer]\ndone", stdout.getvalue())
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_busy_threads = original_get_busy_threads
            bridge.get_thread_ui_name = original_get_thread_ui_name
            bridge.is_thread_busy = original_is_thread_busy
            bridge.get_thread_busy_state = original_get_thread_busy_state
            bridge.start_turn_via_ipc = original_start_turn_via_ipc
            bridge.wait_for_prompt_delivery = original_wait_for_prompt_delivery
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_ipc_ask_allows_sidecar_idle_orphan_without_force(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_threads = bridge.get_busy_threads
        original_get_thread_ui_name = bridge.get_thread_ui_name
        original_is_thread_busy = bridge.is_thread_busy
        original_get_thread_busy_state = bridge.get_thread_busy_state
        original_start_turn_via_ipc = bridge.start_turn_via_ipc
        original_wait_for_prompt_delivery = bridge.wait_for_prompt_delivery
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("old", encoding="utf-8")
                target_thread = bridge.ThreadInfo(
                    id="target-thread",
                    title="Target",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=0,
                )

                bridge.choose_thread = lambda thread_id=None, cwd=None: target_thread
                bridge.get_busy_threads = lambda limit=50: []
                bridge.get_thread_ui_name = lambda thread_id, thread=None: None
                bridge.is_thread_busy = lambda path: True
                bridge.get_thread_busy_state = lambda thread, allow_resume=True: "idle"
                bridge.start_turn_via_ipc = lambda thread, prompt, timeout_sec=10.0, allow_ui_recovery=False: {
                    "owner_client_id": "client-1",
                    "turn_id": "turn-1",
                }
                bridge.wait_for_prompt_delivery = lambda session_offsets, prompt, timeout_sec=4.0: target_thread
                bridge.watch_for_final_answer = lambda **kwargs: {
                    "commentary": [],
                    "final_answer": "done",
                    "status": "ready",
                    "streamed_live": False,
                    "final_streamed_live": False,
                }
                args = argparse.Namespace(
                    thread_id="target-thread",
                    cwd=None,
                    prompt="qa prompt",
                    dry_run=False,
                    force_while_busy=False,
                    ipc=True,
                    ipc_recover_ui=False,
                    background=False,
                    wait=True,
                    timeout=30.0,
                    include_commentary=False,
                    stream=False,
                )
                stdout = io.StringIO()

                with redirect_stdout(stdout):
                    exit_code = bridge.command_ask(args)

            self.assertEqual(exit_code, 0)
            self.assertIn("[final_answer]\ndone", stdout.getvalue())
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_busy_threads = original_get_busy_threads
            bridge.get_thread_ui_name = original_get_thread_ui_name
            bridge.is_thread_busy = original_is_thread_busy
            bridge.get_thread_busy_state = original_get_thread_busy_state
            bridge.start_turn_via_ipc = original_start_turn_via_ipc
            bridge.wait_for_prompt_delivery = original_wait_for_prompt_delivery
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_ipc_ask_pending_delivery_keeps_waiting_for_final(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_threads = bridge.get_busy_threads
        original_get_thread_ui_name = bridge.get_thread_ui_name
        original_is_thread_busy = bridge.is_thread_busy
        original_start_turn_via_ipc = bridge.start_turn_via_ipc
        original_wait_for_prompt_delivery = bridge.wait_for_prompt_delivery
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("", encoding="utf-8")
                target_thread = bridge.ThreadInfo(
                    id="target-thread",
                    title="Target",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=0,
                )
                watched: list[tuple[Path, int]] = []

                bridge.choose_thread = lambda thread_id=None, cwd=None: target_thread
                bridge.get_busy_threads = lambda limit=50: []
                bridge.get_thread_ui_name = lambda thread_id, thread=None: None
                bridge.is_thread_busy = lambda path: False
                bridge.start_turn_via_ipc = lambda thread, prompt, timeout_sec=10.0, allow_ui_recovery=False: {
                    "owner_client_id": "client-1",
                    "turn_id": "turn-1",
                }
                bridge.wait_for_prompt_delivery = lambda session_offsets, prompt, timeout_sec=4.0: None

                def fake_watch_for_final_answer(
                    *,
                    session_path: Path,
                    start_offset: int,
                    timeout_sec: float,
                    include_commentary: bool,
                    stream_live: bool = False,
                    stream_label: str = "",
                    stream_callback=None,
                ) -> final_answer_types.WatchForFinalAnswerResult:
                    watched.append((session_path, start_offset))
                    return {
                        "commentary": [],
                        "final_answer": "done",
                        "status": "final",
                        "streamed_live": False,
                        "final_streamed_live": False,
                    }

                bridge.watch_for_final_answer = fake_watch_for_final_answer
                args = argparse.Namespace(
                    thread_id="target-thread",
                    cwd=None,
                    prompt="qa prompt",
                    dry_run=False,
                    force_while_busy=False,
                    ipc=True,
                    ipc_recover_ui=False,
                    background=False,
                    wait=True,
                    timeout=30.0,
                    include_commentary=False,
                    stream=False,
                )
                stdout = io.StringIO()

                with redirect_stdout(stdout):
                    exit_code = bridge.command_ask(args)

            output = stdout.getvalue()
            self.assertEqual(exit_code, 0)
            self.assertEqual(watched, [(session_path, 0)])
            self.assertIn("[delivery_pending]", output)
            self.assertIn("Continuing to watch for the next Codex reply.", output)
            self.assertIn("[final_answer]\ndone", output)
            self.assertNotIn("Prompt delivery could not be confirmed", output)
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_busy_threads = original_get_busy_threads
            bridge.get_thread_ui_name = original_get_thread_ui_name
            bridge.is_thread_busy = original_is_thread_busy
            bridge.start_turn_via_ipc = original_start_turn_via_ipc
            bridge.wait_for_prompt_delivery = original_wait_for_prompt_delivery
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_busy_ignores_stale_orphan_task_started_after_complete(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_started"}},
                {
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "assistant",
                        "phase": "final_answer",
                        "content": [{"type": "output_text", "text": "done"}],
                    },
                },
                {"type": "event_msg", "payload": {"type": "task_complete"}},
                {"type": "event_msg", "payload": {"type": "task_started"}},
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            old_time = time.time() - 120.0
            os.utime(session_path, (old_time, old_time))

            with EnvPatch("CODEX_BRIDGE_ORPHAN_TASK_STARTED_GRACE_SECONDS", "60"):
                self.assertFalse(bridge.is_thread_busy(session_path))

    def test_bridge_busy_keeps_fresh_orphan_task_started_busy(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_complete"}},
                {"type": "event_msg", "payload": {"type": "task_started"}},
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )

            with EnvPatch("CODEX_BRIDGE_ORPHAN_TASK_STARTED_GRACE_SECONDS", "60"):
                self.assertTrue(bridge.is_thread_busy(session_path))

    def test_bridge_busy_ignores_old_unfinished_noninteractive_session(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_started"}},
                {"type": "event_msg", "payload": {"type": "user_message", "message": "run"}},
                {
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "phase": "commentary",
                        "message": "working",
                    },
                },
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            old_time = time.time() - 3600.0
            os.utime(session_path, (old_time, old_time))

            with EnvPatch("CODEX_BRIDGE_STALE_BUSY_SESSION_SECONDS", "1800"):
                self.assertFalse(bridge.is_thread_busy(session_path))

    def test_bridge_busy_keeps_old_pending_interactive_session_busy(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_started"}},
                {
                    "type": "response_item",
                    "payload": {
                        "type": "function_call",
                        "name": "request_user_input",
                        "call_id": "call-1",
                        "arguments": "{}",
                    },
                },
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            old_time = time.time() - 3600.0
            os.utime(session_path, (old_time, old_time))

            with EnvPatch("CODEX_BRIDGE_STALE_BUSY_SESSION_SECONDS", "1800"):
                self.assertTrue(bridge.is_thread_busy(session_path))

    def test_bridge_busy_state_trusts_sidecar_idle_for_noninteractive_orphan(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_started"}},
                {"type": "event_msg", "payload": {"type": "user_message", "message": "stuck"}},
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            old_time = time.time() - 120.0
            os.utime(session_path, (old_time, old_time))
            thread = bridge.ThreadInfo(
                id="thread-1",
                title="title",
                cwd=str(temp_dir),
                updated_at=1,
                rollout_path=str(session_path),
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )
            client = IdleSidecarClient()

            with EnvPatch("CODEX_BRIDGE_ORPHAN_TASK_STARTED_GRACE_SECONDS", "60"):
                self.assertTrue(bridge.is_thread_busy(session_path))
                self.assertEqual(bridge.get_thread_busy_state(thread, client=client), "idle")

    def test_bridge_busy_state_keeps_interactive_even_if_sidecar_idle(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            session_path = Path(temp_dir) / "session.jsonl"
            events = [
                {"type": "event_msg", "payload": {"type": "task_started"}},
                {
                    "type": "response_item",
                    "payload": {
                        "type": "function_call",
                        "name": "request_user_input",
                        "call_id": "call-1",
                        "arguments": "{}",
                    },
                },
            ]
            session_path.write_text(
                "\n".join(json.dumps(event) for event in events) + "\n",
                encoding="utf-8",
            )
            thread = bridge.ThreadInfo(
                id="thread-1",
                title="title",
                cwd=str(temp_dir),
                updated_at=1,
                rollout_path=str(session_path),
                model="gpt",
                reasoning_effort="high",
                tokens_used=1,
            )
            client = IdleSidecarClient()

            self.assertEqual(bridge.get_thread_busy_state(thread, client=client), "waiting-input")

    def test_get_busy_threads_excludes_sidecar_idle_orphan(self) -> None:
        original_load_recent_threads = bridge.load_recent_threads
        original_sidecar = bridge.CodexAppServerSidecar
        try:
            class FakeSidecar:
                def read_thread(self, thread_id, include_turns=False):
                    return {"thread": {"status": {"type": "idle"}}}

                def close(self):
                    pass

            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                events = [
                    {"type": "event_msg", "payload": {"type": "task_started"}},
                    {"type": "event_msg", "payload": {"type": "user_message", "message": "stuck"}},
                ]
                session_path.write_text(
                    "\n".join(json.dumps(event) for event in events) + "\n",
                    encoding="utf-8",
                )
                old_time = time.time() - 120.0
                os.utime(session_path, (old_time, old_time))
                thread = bridge.ThreadInfo(
                    id="thread-1",
                    title="title",
                    cwd=str(temp_dir),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=1,
                )
                bridge.load_recent_threads = lambda limit=50: [thread]
                bridge.CodexAppServerSidecar = FakeSidecar

                with EnvPatch("CODEX_BRIDGE_ORPHAN_TASK_STARTED_GRACE_SECONDS", "60"):
                    self.assertEqual(bridge.get_busy_threads(), [])
        finally:
            bridge.load_recent_threads = original_load_recent_threads
            bridge.CodexAppServerSidecar = original_sidecar

    def test_bridge_archive_retries_windows_lock_after_stopping_codex(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_busy_state = bridge.get_thread_busy_state
        original_archive_once = bridge.archive_thread_once
        original_stop_candidates = bridge.stop_codex_archive_lock_candidates
        original_wait_record = bridge.wait_for_thread_record
        original_selected = bridge.get_selected_thread_id
        original_sync = bridge.sync_session_index_with_state
        calls: list[str] = []
        stop_calls: list[bool] = []

        thread = bridge.ThreadInfo(
            id="thread-1",
            title="Locked Archive",
            cwd="C:\\repo",
            updated_at=1,
            rollout_path="C:\\Users\\banpo\\.codex\\sessions\\thread-1.jsonl",
            model="gpt",
            reasoning_effort="high",
            tokens_used=0,
        )

        def fake_archive_once(thread_id: str) -> None:
            calls.append(thread_id)
            if len(calls) == 1:
                raise bridge.CodexSidecarError(
                    "thread/archive failed: failed to archive thread: "
                    "다른 프로세스가 파일을 사용 중이기 때문에 프로세스가 액세스 할 수 없습니다. (os error 32)"
                )

        try:
            bridge.choose_thread = lambda thread_id=None, cwd=None: thread
            bridge.get_thread_busy_state = lambda item, allow_resume=True: "idle"
            bridge.archive_thread_once = fake_archive_once
            bridge.stop_codex_archive_lock_candidates = lambda: stop_calls.append(True) or [
                "codex_desktop_stop: stopped=True source=test exe=Codex.exe",
                "codex_app_server_stop: stopped=True",
            ]
            bridge.wait_for_thread_record = lambda thread_id, archived=None, timeout_sec=8.0: (thread, True)
            bridge.get_selected_thread_id = lambda: None
            bridge.sync_session_index_with_state = lambda: None

            output = io.StringIO()
            args = argparse.Namespace(
                thread_ref=None,
                thread_id="thread-1",
                cwd=None,
                timeout=0.0,
                no_kill_codex_on_lock=False,
            )
            with redirect_stdout(output):
                self.assertEqual(bridge.command_archive(args), 0)

            self.assertEqual(calls, ["thread-1", "thread-1"])
            self.assertEqual(stop_calls, [True])
            text = output.getvalue()
            self.assertIn("archive_lock_error: thread/archive failed", text)
            self.assertIn("archive_lock_retry: stopping Codex processes and retrying once", text)
            self.assertIn("archive_lock_retry: succeeded", text)
            self.assertIn("archived_thread: thread-1", text)
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_busy_state = original_get_busy_state
            bridge.archive_thread_once = original_archive_once
            bridge.stop_codex_archive_lock_candidates = original_stop_candidates
            bridge.wait_for_thread_record = original_wait_record
            bridge.get_selected_thread_id = original_selected
            bridge.sync_session_index_with_state = original_sync

    def test_windows_harness_preflight_does_not_check_target_busy(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            thread = bridge.ThreadInfo(
                id="target-thread",
                title="Target",
                cwd=str(temp_dir),
                updated_at=10,
                rollout_path=str(Path(temp_dir) / "session.jsonl"),
                model="gpt",
                reasoning_effort="high",
                tokens_used=0,
            )

            fake_bridge = PreflightTargetBridge(
                thread,
                "preflight_ask should not inspect idle/busy state",
            )

            preflight = harness.preflight_ask("target-thread", bridge_module=fake_bridge, now=1.0)

        self.assertTrue(preflight.accepted)
        self.assertEqual(preflight.route, "ask")
        self.assertEqual(preflight.target_state, "not_checked")
        self.assertFalse(preflight.can_steer)
        self.assertEqual(preflight.not_sent_reason, "")

    def test_windows_harness_preflight_ignores_other_busy_threads(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            target = bridge.ThreadInfo(
                id="target-thread",
                title="Target",
                cwd=str(temp_dir),
                updated_at=10,
                rollout_path=str(Path(temp_dir) / "target.jsonl"),
                model="gpt",
                reasoning_effort="high",
                tokens_used=0,
            )

            fake_bridge = PreflightTargetBridge(
                target,
                "preflight_ask should not inspect any busy threads",
            )

            preflight = harness.preflight_ask("target-thread", bridge_module=fake_bridge, now=1.0)

        self.assertTrue(preflight.accepted)
        self.assertEqual(preflight.route, "ask")
        self.assertEqual(preflight.target_state, "not_checked")
        self.assertFalse(preflight.can_steer)
        self.assertEqual(preflight.not_sent_reason, "")

    def test_windows_harness_runtime_reports_desktop_available(self) -> None:
        fake_bridge = DesktopDiscoveryStub(
            (Path("C:/Codex/Codex.exe"), "fake")
        )

        with mock.patch.object(harness, "probe_codex_cli", return_value=("codex.exe", "ok")):
            status = harness.get_runtime_status(bridge_module=fake_bridge)

        self.assertEqual(status.platform, "windows-local")
        self.assertEqual(status.codex_cli_status, "ok")
        self.assertEqual(status.codex_desktop_status, "available")

    def test_windows_harness_runtime_reports_desktop_not_found(self) -> None:
        fake_bridge = DesktopDiscoveryStub()

        with mock.patch.object(harness, "probe_codex_cli", return_value=("", "not_found")):
            status = harness.get_runtime_status(bridge_module=fake_bridge)

        self.assertEqual(status.codex_cli_status, "not_found")
        self.assertEqual(status.codex_desktop_status, "not_found")

    def test_windows_harness_runtime_reports_desktop_unavailable_on_error(self) -> None:
        fake_bridge = DesktopDiscoveryStub(failure=RuntimeError("probe failed"))

        with mock.patch.object(harness, "probe_codex_cli", return_value=("", "permission_denied")):
            status = harness.get_runtime_status(bridge_module=fake_bridge)

        self.assertEqual(status.codex_cli_status, "permission_denied")
        self.assertEqual(status.codex_desktop_status, "unavailable")

    def test_choice_views_claim_once_and_disable_buttons(self) -> None:
        approval_view = bot.ApprovalView("thread-1")
        approval_custom_ids = {
            getattr(item, "label", ""): getattr(item, "custom_id", "")
            for item in approval_view.children
        }
        self.assertEqual(approval_custom_ids["Approve"], "codex_approval:thread-1:1")
        self.assertEqual(approval_custom_ids["Approve session"], "codex_approval:thread-1:2")
        self.assertEqual(approval_custom_ids["Reject"], "codex_approval:thread-1:3")
        self.assertEqual(approval_custom_ids["Cancel"], "codex_approval:thread-1:cancel")

        input_view = bot.InputChoiceView("thread-1", [("1", "First"), ("2", "Second")])
        input_custom_ids = {
            getattr(item, "label", ""): getattr(item, "custom_id", "")
            for item in input_view.children
        }
        self.assertEqual(bot.parse_input_choice_custom_id(input_custom_ids["First"]), ("thread-1", "1"))
        self.assertEqual(bot.parse_input_choice_custom_id(input_custom_ids["Second"]), ("thread-1", "2"))
        self.assertTrue(input_view.claim())
        self.assertFalse(input_view.claim())
        self.assertTrue(all(getattr(item, "disabled", False) for item in input_view.children))

        message = ReturningTargetMessage()
        message.author = FakeAuthor(author_id=1)
        busy_view = bot.BusyChoiceView(message, "prompt", target_thread_id="thread-1")
        self.assertTrue(busy_view.claim())
        self.assertFalse(busy_view.claim())
        self.assertTrue(all(getattr(item, "disabled", False) for item in busy_view.children))

    async def test_approval_view_message_edit_runtime_failure_logs_and_submits(self) -> None:
        original_submit = bot.submit_approval_reply
        submitted: list[tuple[str, str]] = []
        try:
            bot.submit_approval_reply = lambda target_thread_id, answer: (
                submitted.append((target_thread_id, answer)) or (0, "approved")
            )
            interaction = DiscordFakeInteraction(
                command_name="-",
                channel_id=222,
                message_failure=RuntimeError("approval edit unavailable"),
            )
            view = bot.ApprovalView("thread-1")

            await view._submit(interaction, "1")

            log_text = Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")
            self.assertEqual(submitted, [("thread-1", "1")])
            self.assertIn("Approval submitted\n\napproved", interaction.fake_followup.messages)
            self.assertIn("approval_button_message_edit_failed", log_text)
        finally:
            bot.submit_approval_reply = original_submit

    async def test_approval_view_message_edit_type_error_is_not_edit_failed(self) -> None:
        original_submit = bot.submit_approval_reply
        submitted: list[tuple[str, str]] = []
        try:
            bot.submit_approval_reply = lambda target_thread_id, answer: (
                submitted.append((target_thread_id, answer)) or (0, "approved")
            )
            interaction = DiscordFakeInteraction(
                command_name="-",
                channel_id=222,
                message_failure=TypeError("bad approval edit dependency"),
            )
            view = bot.ApprovalView("thread-1")

            with self.assertRaisesRegex(TypeError, "bad approval edit dependency"):
                await view._submit(interaction, "1")

            log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
            self.assertEqual(submitted, [])
            self.assertNotIn("approval_button_message_edit_failed", log_text)
        finally:
            bot.submit_approval_reply = original_submit

    async def test_input_choice_button_message_edit_runtime_failure_logs_and_submits(self) -> None:
        original_submit = bot.submit_input_reply
        submitted: list[tuple[str, str]] = []
        try:
            bot.submit_input_reply = lambda target_thread_id, value: (
                submitted.append((target_thread_id, value)) or (0, "answered")
            )
            interaction = DiscordFakeInteraction(
                command_name="-",
                channel_id=222,
                message_failure=RuntimeError("input edit unavailable"),
            )
            view = bot.InputChoiceView("thread-1", [("choice-1", "First")])
            button = view.children[0]

            await button.callback(interaction)

            log_text = Path(os.environ["CODEX_DISCORD_LOG_PATH"]).read_text(encoding="utf-8")
            self.assertEqual(submitted, [("thread-1", "choice-1")])
            self.assertIn("Input submitted\n\nanswered", interaction.fake_followup.messages)
            self.assertIn("input_choice_button_message_edit_failed", log_text)
        finally:
            bot.submit_input_reply = original_submit

    async def test_input_choice_button_message_edit_type_error_is_not_edit_failed(self) -> None:
        original_submit = bot.submit_input_reply
        submitted: list[tuple[str, str]] = []
        try:
            bot.submit_input_reply = lambda target_thread_id, value: (
                submitted.append((target_thread_id, value)) or (0, "answered")
            )
            interaction = DiscordFakeInteraction(
                command_name="-",
                channel_id=222,
                message_failure=TypeError("bad input edit dependency"),
            )
            view = bot.InputChoiceView("thread-1", [("choice-1", "First")])
            button = view.children[0]

            with self.assertRaisesRegex(TypeError, "bad input edit dependency"):
                await button.callback(interaction)

            log_path = Path(os.environ["CODEX_DISCORD_LOG_PATH"])
            log_text = log_path.read_text(encoding="utf-8") if log_path.exists() else ""
            self.assertEqual(submitted, [])
            self.assertNotIn("input_choice_button_message_edit_failed", log_text)
        finally:
            bot.submit_input_reply = original_submit

    async def test_busy_choice_denied_user_is_logged(self) -> None:
        message = FakeMessage()
        view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
        interaction = DiscordFakeInteraction(command_name="ask", channel_id=222, user_id=999)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                allowed = await view.interaction_check(interaction)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertFalse(allowed)
        self.assertEqual(interaction.fake_response.messages, ["Only the original sender can choose this."])
        self.assertIn("busy_choice_denied user=999 owner=242286902982606848 target=thread-1", log_text)

    async def test_busy_choice_duplicate_click_is_logged(self) -> None:
        message = FakeMessage()
        view = bot.BusyChoiceView(message, "please steer", target_thread_id="thread-1")
        self.assertTrue(view.claim())
        button = next(item for item in view.children if getattr(item, "label", "") == "Queue next")
        interaction = DiscordFakeInteraction(command_name="ask", channel_id=222)

        with tempfile.TemporaryDirectory() as temp_dir:
            log_path = Path(temp_dir) / "discord-smoke.log"
            with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                await button.callback(interaction)
            log_text = log_path.read_text(encoding="utf-8")

        self.assertEqual(interaction.fake_response.messages, ["This busy choice was already handled."])
        self.assertIn("busy_choice_already_handled action=queue_next", log_text)
        self.assertIn("target=thread-1", log_text)

    async def test_busy_choice_steer_defers_before_persistent_claim(self) -> None:
        original_claim_busy_choice_record = bot.claim_busy_choice_record
        observed_deferred: list[bool] = []
        message = FakeMessage()
        view = bot.BusyChoiceView(
            message,
            "please steer",
            target_thread_id="thread-1",
            choice_id="0123456789abcdef01234567",
        )
        button = next(item for item in view.children if getattr(item, "label", "") == "Steer now")
        interaction = DiscordFakeInteraction(command_name="ask", channel_id=222)

        try:
            def fake_claim_busy_choice_record(choice_id: str) -> bool:
                observed_deferred.append(interaction.fake_response.deferred)
                return False

            bot.claim_busy_choice_record = fake_claim_busy_choice_record
            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await button.callback(interaction)
                log_text = log_path.read_text(encoding="utf-8")
        finally:
            bot.claim_busy_choice_record = original_claim_busy_choice_record

        self.assertEqual(observed_deferred, [True])
        self.assertTrue(interaction.fake_response.deferred)
        self.assertEqual(interaction.fake_followup.messages, ["This busy choice was already handled."])
        self.assertIn("busy_choice_already_handled action=steer_now", log_text)

    def test_fit_single_message_truncates_to_discord_limit(self) -> None:
        fitted = bot.fit_single_message("x" * 4100)
        self.assertLessEqual(len(fitted), bot.DISCORD_MAX_LEN)
        self.assertTrue(fitted.endswith("[truncated for Discord]"))

    def test_format_discord_command_label_truncates_and_flattens(self) -> None:
        label = bot.format_discord_command_label("x" * 100 + "\nboom")
        self.assertLessEqual(len(label), 80)
        self.assertNotIn("\n", label)
        self.assertTrue(label.endswith("..."))

    async def test_discord_button_qa_exercises_button_handlers(self) -> None:
        old_db_path = bot.MIRROR_DB_PATH
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as temp_dir:
            bot.MIRROR_DB_PATH = Path(temp_dir) / "mirror.sqlite"
            try:
                fake_bot = SimpleNamespace()
                message = ReturningTargetMessage(channel_id=222)
                log_path = Path(temp_dir) / "discord-smoke.log"

                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    output = await bot.run_discord_button_qa(fake_bot, message)
                log_text = log_path.read_text(encoding="utf-8")
            finally:
                bot.MIRROR_DB_PATH = old_db_path

        self.assertIn("ignore: ok", output)
        self.assertIn("claimed_record: ok", output)
        self.assertIn("missing_record: ok", output)
        self.assertIn("stale_cleanup: ok", output)
        self.assertIn("steer_success: ok", output)
        self.assertIn("approval_persistent: ok", output)
        self.assertIn("input_choice_persistent: ok", output)
        self.assertIn("result: ok", output)
        self.assertEqual(len(message.channel.sent_messages), 8)
        self.assertEqual(
            [content for content, _view in message.channel.messages].count(
                "Discord steering submitted.\nmessage: QA button steer success smoke"
            ),
            1,
        )
        self.assertEqual(sum(1 for sent in message.channel.sent_messages if sent.edits == [None]), 7)
        self.assertEqual(sum(1 for sent in message.channel.sent_messages if sent.edits == []), 1)
        self.assertIn("button_qa_done channel=222 user=242286902982606848 result=ok", log_text)

    async def test_on_message_ignores_bot_authored_korean_completion_packet(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_handle_plain_ask = bot.handle_plain_ask
        try:
            async def fail_handle_plain_ask(message, prompt, *, target_thread_id=None):
                raise AssertionError("bot bridge completion packets must not reach Codex")

            bot.get_mirrored_codex_thread_id = lambda channel_id: None
            bot.handle_plain_ask = fail_handle_plain_ask
            client = self._make_message_test_client(
                allowed_user_ids={1500506752234422322},
                plain_ask_mention_user_ids={1511380398914142379},
            )
            message = FakeMessage(
                content="<@1511380398914142379>\n완료: restart-check handoff 전달 완료.",
                channel_id=333,
            )
            message.author = FakeAuthor(1500506752234422322, bot=True)
            message.raw_mentions = [1511380398914142379]
            message.mentions = [SimpleNamespace(id=1511380398914142379, bot=True)]

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await bot.CodexDiscordBot.on_message(client, message)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(message.channel.messages, [])
            self.assertIn("ignored_message reason=bot_bridge_operational_packet", log_text)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.handle_plain_ask = original_handle_plain_ask

    async def test_on_message_context_fallback_ignores_unmentioned_plain_chatter(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_handle_plain_ask = bot.handle_plain_ask
        try:
            async def fail_handle_plain_ask(message, prompt, *, target_thread_id=None):
                raise AssertionError("plain chatter must not reach Codex")

            bot.get_mirrored_codex_thread_id = lambda channel_id: None
            bot.handle_plain_ask = fail_handle_plain_ask
            client = self._make_message_test_client(
                allowed_user_ids={242286902982606848},
                plain_ask_mention_user_ids={1500506752234422322},
            )
            message = FakeMessage(content="plain channel chatter", channel_id=333)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with (
                    EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                    EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "1"),
                    EnvPatch(
                        "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
                        message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
                    ),
                ):
                    await bot.CodexDiscordBot.on_message(client, message)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(message.channel.messages, [])
            self.assertIn("ignored_message reason=required_mention_missing chat=333", log_text)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.handle_plain_ask = original_handle_plain_ask

    async def test_on_message_context_fallback_ignores_other_bot_mention_without_bridge_context(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_handle_plain_ask = bot.handle_plain_ask
        try:
            async def fail_handle_plain_ask(message, prompt, *, target_thread_id=None):
                raise AssertionError("other bot mentions without bridge context must not reach Codex")

            bot.get_mirrored_codex_thread_id = lambda channel_id: None
            bot.handle_plain_ask = fail_handle_plain_ask
            client = self._make_message_test_client(
                allowed_user_ids={242286902982606848},
                plain_ask_mention_user_ids={1511380398914142379},
            )
            message = FakeMessage(content="<@1500506752234422322> ping", channel_id=333)
            message.raw_mentions = [1500506752234422322]
            message.mentions = [SimpleNamespace(id=1500506752234422322)]

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with (
                    EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                    EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "1"),
                    EnvPatch(
                        "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
                        message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
                    ),
                ):
                    await bot.CodexDiscordBot.on_message(client, message)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(message.channel.messages, [])
            self.assertIn("ignored_message reason=required_mention_missing chat=333", log_text)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.handle_plain_ask = original_handle_plain_ask

    async def test_on_message_context_fallback_accepts_other_bot_mention_with_bridge_context(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_describe_project = bot.describe_mirrored_project_channel
        original_handle_plain_ask = bot.handle_plain_ask
        original_is_thread_runner_busy = bot.is_thread_runner_busy
        original_get_busy_state = bot.get_busy_state_for_thread
        calls: list[tuple[object, str, str | None]] = []
        try:
            bot.get_mirrored_codex_thread_id = lambda channel_id: None
            bot.describe_mirrored_project_channel = lambda channel_id: None
            bot.get_busy_state_for_thread = lambda target_thread_id: ("idle", None, "")

            async def runner_idle(target_thread_id):
                return False

            async def fake_handle_plain_ask(message, prompt, *, target_thread_id=None):
                calls.append((message, prompt, target_thread_id))

            bot.is_thread_runner_busy = runner_idle
            bot.handle_plain_ask = fake_handle_plain_ask
            client = self._make_message_test_client(
                allowed_user_ids={242286902982606848},
                plain_ask_mention_user_ids={1511380398914142379},
            )
            message = FakeMessage(
                content="<@1500506752234422322> 잘아타스한테 티켓 다시 보내봐",
                channel_id=333,
            )
            message.raw_mentions = [1500506752234422322]
            message.mentions = [SimpleNamespace(id=1500506752234422322)]

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with (
                    EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)),
                    EnvPatch("DISCORD_PLAIN_ASK_CONTEXT_FALLBACK", "1"),
                    EnvPatch(
                        "DISCORD_PLAIN_ASK_CONTEXT_KEYWORDS",
                        message_gate.DEFAULT_PLAIN_ASK_CONTEXT_KEYWORDS,
                    ),
                ):
                    await bot.CodexDiscordBot.on_message(client, message)
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(calls, [(message, message.content, None)])
            self.assertIn("plain_ask_context_fallback chat=333", log_text)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.describe_mirrored_project_channel = original_describe_project
            bot.handle_plain_ask = original_handle_plain_ask
            bot.is_thread_runner_busy = original_is_thread_runner_busy
            bot.get_busy_state_for_thread = original_get_busy_state

    async def test_on_message_project_parent_response_is_chunked(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_describe_project = bot.describe_mirrored_project_channel
        original_handle_plain_ask = bot.handle_plain_ask
        original_is_thread_runner_busy = bot.is_thread_runner_busy
        original_get_busy_state = bot.get_busy_state_for_thread
        try:
            bot.get_mirrored_codex_thread_id = lambda channel_id: None
            bot.describe_mirrored_project_channel = lambda channel_id: "x" * 4100
            bot.get_busy_state_for_thread = lambda target_thread_id: ("idle", None, "")

            async def runner_idle(target_thread_id):
                return False

            async def fail_handle_plain_ask(message, prompt, *, target_thread_id=None):
                raise AssertionError("project parent messages must not fall back to selected thread")

            bot.is_thread_runner_busy = runner_idle
            bot.handle_plain_ask = fail_handle_plain_ask
            client = self._make_message_test_client(
                allowed_user_ids={242286902982606848},
            )
            message = FakeMessage(content="please hook", channel_id=333)

            with tempfile.TemporaryDirectory() as temp_dir:
                log_path = Path(temp_dir) / "discord-smoke.log"
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await bot.CodexDiscordBot.on_message(client, message)

            sent = [content for content, _view in message.channel.messages]
            self.assertGreater(len(sent), 1)
            self.assertTrue(all(len(content) <= bot.DISCORD_MAX_LEN for content in sent))
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.describe_mirrored_project_channel = original_describe_project
            bot.handle_plain_ask = original_handle_plain_ask
            bot.is_thread_runner_busy = original_is_thread_runner_busy
            bot.get_busy_state_for_thread = original_get_busy_state

    def test_build_context_refresh_message_includes_recent_bounded_items(self) -> None:
        original_get_mirrored = bot.get_mirrored_codex_thread_id
        original_resolve_selected = bot.resolve_selected_target
        original_choose_thread = bridge.choose_thread
        try:
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                events = [
                    {
                        "type": "event_msg",
                        "timestamp": "2026-06-08T00:00:00Z",
                        "payload": {"type": "user_message", "message": "older user"},
                    },
                    {
                        "type": "response_item",
                        "timestamp": "2026-06-08T00:00:01Z",
                        "payload": {
                            "type": "message",
                            "role": "user",
                            "content": [{"type": "input_text", "text": "latest question"}],
                        },
                    },
                    {
                        "type": "response_item",
                        "timestamp": "2026-06-08T00:00:02Z",
                        "payload": {
                            "type": "message",
                            "role": "assistant",
                            "phase": "final_answer",
                            "content": [{"type": "output_text", "text": "latest answer"}],
                        },
                    },
                ]
                session_path.write_text(
                    "\n".join(json.dumps(event, ensure_ascii=True) for event in events) + "\n",
                    encoding="utf-8",
                )
                thread = bridge.ThreadInfo(
                    id="thread-1",
                    title="Thread",
                    cwd=str(Path(temp_dir)),
                    updated_at=1,
                    rollout_path=str(session_path),
                    model="gpt",
                    reasoning_effort="high",
                    tokens_used=1,
                )
                bot.get_mirrored_codex_thread_id = lambda channel_id: "thread-1"
                bot.resolve_selected_target = lambda: (None, "")
                bridge.choose_thread = lambda thread_id=None, cwd=None: thread

                output = bot.build_context_refresh_message(222, limit=2)
        finally:
            bot.get_mirrored_codex_thread_id = original_get_mirrored
            bot.resolve_selected_target = original_resolve_selected
            bridge.choose_thread = original_choose_thread

        self.assertIn("Context refresh", output)
        self.assertIn("items: 2/2", output)
        self.assertIn("[user]\nlatest question", output)
        self.assertIn("[assistant final]\nlatest answer", output)
        self.assertNotIn("older user", output)

    async def test_delete_stale_project_channels_deletes_mirror_text_channel(self) -> None:
        original_text_channel = bot.discord.TextChannel

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id = 111
                self.category_id = 999
                self.topic = "Codex project mirror: stale"
                self.deleted_reasons: list[str] = []

            async def delete(self, reason: str) -> None:
                self.deleted_reasons.append(reason)

        channel = FakeTextChannel()
        guild = ProjectCleanupGuildStub(channel)
        category = ProjectCleanupCategoryStub(999)
        try:
            bot.discord.TextChannel = FakeTextChannel
            result = await bot.delete_stale_project_channels(
                guild,
                category,
                [("stale-project", "stale", 111)],
            )
        finally:
            bot.discord.TextChannel = original_text_channel

        self.assertEqual(result["deleted"], 1)
        self.assertEqual(result["missing"], 0)
        self.assertEqual(result["skipped"], 0)
        self.assertEqual(result["failed"], 0)
        self.assertEqual(len(channel.deleted_reasons), 1)
        self.assertIn("stale-project", channel.deleted_reasons[0])

    async def test_delete_stale_project_channels_skips_non_mirror_text_channel(self) -> None:
        original_text_channel = bot.discord.TextChannel

        class FakeTextChannel:
            def __init__(self) -> None:
                self.id = 111
                self.category_id = 123
                self.topic = "general"
                self.deleted = False

            async def delete(self, reason: str) -> None:
                self.deleted = True

        channel = FakeTextChannel()
        guild = ProjectCleanupGuildStub(channel)
        category = ProjectCleanupCategoryStub(999)
        try:
            bot.discord.TextChannel = FakeTextChannel
            result = await bot.delete_stale_project_channels(
                guild,
                category,
                [("stale-project", "stale", 111)],
            )
        finally:
            bot.discord.TextChannel = original_text_channel

        self.assertEqual(result["deleted"], 0)
        self.assertEqual(result["skipped"], 1)
        self.assertFalse(channel.deleted)

    async def test_discord_ask_relay_sends_quiet_progress_notice(self) -> None:
        target = FakeTarget()
        relay = bot.DiscordAskRelay(
            asyncio.get_running_loop(),
            target,
            "thread-1",
            "project:1",
            quiet_notice_delay_sec=0.01,
        )

        relay.feed_line("[waiting_for_final_answer]")
        await asyncio.sleep(0.05)
        await asyncio.to_thread(relay.finish)

        self.assertEqual(len(target.messages), 1)
        self.assertIn("Codex is still working.", target.messages[0][0])
        self.assertIn("compacts context", target.messages[0][0])
        self.assertTrue(relay.quiet_notice_sent)
        self.assertFalse(relay.sent_live)

    async def test_discord_ask_relay_default_quiet_progress_notice_is_disabled(self) -> None:
        target = FakeTarget()
        relay = bot.DiscordAskRelay(
            asyncio.get_running_loop(),
            target,
            "thread-1",
            "project:1",
        )

        relay.feed_line("[waiting_for_final_answer]")
        await asyncio.sleep(0.05)
        await asyncio.to_thread(relay.finish)

        self.assertEqual(target.messages, [])
        self.assertFalse(relay.quiet_notice_sent)
        self.assertFalse(relay.sent_live)

    async def test_discord_ask_relay_cancels_quiet_notice_after_final(self) -> None:
        target = FakeTarget()
        relay = bot.DiscordAskRelay(
            asyncio.get_running_loop(),
            target,
            "thread-1",
            "project:1",
            quiet_notice_delay_sec=0.05,
        )

        relay.feed_line("[waiting_for_final_answer]")
        relay.feed_line("[final_answer]")
        relay.feed_line("done")
        await asyncio.to_thread(relay.finish)
        await asyncio.sleep(0.08)

        self.assertEqual(target.messages, [("Final\n\ndone", None)])
        self.assertFalse(relay.quiet_notice_sent)
        self.assertTrue(relay.sent_live)

    async def test_discord_ask_relay_streams_commentary_before_final_by_default(self) -> None:
        original_send_chunks = bot.send_chunks
        sent: list[str] = []
        try:
            async def fake_send_chunks(channel, text):
                sent.append(text)

            bot.send_chunks = fake_send_chunks
            target = FakeTarget()
            relay = bot.DiscordAskRelay(
                asyncio.get_running_loop(),
                target,
                "thread-1",
                "project:1",
                quiet_notice_delay_sec=-1,
            )

            relay.feed_line("[commentary]")
            relay.feed_line("checking order")
            relay.feed_line("[final_answer]")
            relay.feed_line("done")
            relay.feed_line("[ready]")
            await asyncio.to_thread(relay.finish)

            self.assertEqual(sent, ["In progress\n\nchecking order", "Final\n\ndone"])
            self.assertTrue(relay.sent_live)
            self.assertTrue(relay.saw_final)
        finally:
            bot.send_chunks = original_send_chunks

    async def test_discord_ask_relay_can_suppress_commentary_before_final(self) -> None:
        original_send_chunks = bot.send_chunks
        sent: list[str] = []
        try:
            async def fake_send_chunks(channel, text):
                sent.append(text)

            bot.send_chunks = fake_send_chunks
            target = FakeTarget()
            relay = bot.DiscordAskRelay(
                asyncio.get_running_loop(),
                target,
                "thread-1",
                "project:1",
                quiet_notice_delay_sec=-1,
                send_commentary_blocks=False,
            )

            relay.feed_line("[commentary]")
            relay.feed_line("checking order")
            relay.feed_line("[final_answer]")
            relay.feed_line("done")
            relay.feed_line("[ready]")
            await asyncio.to_thread(relay.finish)

            self.assertEqual(sent, ["Final\n\ndone"])
            self.assertTrue(relay.sent_live)
            self.assertTrue(relay.saw_final)
        finally:
            bot.send_chunks = original_send_chunks

    async def test_steering_watch_streams_final_to_discord_relay(self) -> None:
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            def fake_watch_for_final_answer(
                session_path,
                start_offset,
                timeout_sec,
                include_commentary,
                stream_live=False,
                stream_label="",
                stream_callback=None,
            ):
                if stream_live and stream_callback is not None:
                    for line in ["[final_answer]", "steered done", ""]:
                        stream_callback(line)
                return {
                    "status": "final",
                    "commentary": [],
                    "final_answer": "steered done",
                    "streamed_live": True,
                    "final_streamed_live": True,
                }

            bridge.watch_for_final_answer = fake_watch_for_final_answer
            target = FakeTarget()
            relay = bot.DiscordAskRelay(
                asyncio.get_running_loop(),
                target,
                "thread-1",
                "project:1",
                quiet_notice_delay_sec=0.05,
            )
            steering_result = bot.SteeringPromptResult(
                0,
                "Steering sent",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            exit_code, _output = await asyncio.to_thread(
                bot.run_steering_watch_stream,
                steering_result,
                relay,
                timeout_sec=1,
            )

            self.assertEqual(exit_code, 0)
            self.assertEqual(target.messages, [("Final\n\nsteered done", None)])
            self.assertTrue(relay.sent_live)
            self.assertTrue(relay.saw_final)
        finally:
            bridge.watch_for_final_answer = original_watch_for_final_answer

    async def test_steering_watch_streams_commentary_before_final_by_default(self) -> None:
        original_watch_for_final_answer = bridge.watch_for_final_answer
        try:
            def fake_watch_for_final_answer(
                session_path,
                start_offset,
                timeout_sec,
                include_commentary,
                stream_live=False,
                stream_label="",
                stream_callback=None,
            ):
                if stream_live and stream_callback is not None:
                    for line in ["[commentary]", "checking files", "", "[final_answer]", "done", ""]:
                        stream_callback(line)
                return {
                    "status": "final",
                    "commentary": ["checking files"],
                    "final_answer": "done",
                    "streamed_live": True,
                    "final_streamed_live": True,
                }

            bridge.watch_for_final_answer = fake_watch_for_final_answer
            target = FakeTarget()
            relay = bot.DiscordAskRelay(
                asyncio.get_running_loop(),
                target,
                "thread-1",
                "project:1",
                quiet_notice_delay_sec=0.05,
            )
            steering_result = bot.SteeringPromptResult(
                0,
                "Steering sent",
                target_thread_id="thread-1",
                target_ref="project:1",
                session_path="session.jsonl",
                start_offset=10,
            )

            exit_code, _output = await asyncio.to_thread(
                bot.run_steering_watch_stream,
                steering_result,
                relay,
                timeout_sec=1,
            )

            self.assertEqual(exit_code, 0)
            self.assertEqual(
                target.messages,
                [("In progress\n\nchecking files", None), ("Final\n\ndone", None)],
            )
            self.assertTrue(relay.sent_live)
            self.assertTrue(relay.saw_final)
        finally:
            bridge.watch_for_final_answer = original_watch_for_final_answer

    def test_bridge_marks_final_streamed_live_separately(self) -> None:
        original_read_new_session_events = bridge.read_new_session_events
        try:
            def fake_read_new_session_events(session_path, cursor):
                if cursor:
                    return [], cursor
                return [
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "message",
                            "role": "assistant",
                            "phase": "commentary",
                            "content": [{"type": "output_text", "text": "working"}],
                        },
                    },
                    {
                        "type": "response_item",
                        "payload": {
                            "type": "message",
                            "role": "assistant",
                            "phase": "final_answer",
                            "content": [{"type": "output_text", "text": "done"}],
                        },
                    },
                    {
                        "type": "event_msg",
                        "payload": {
                            "type": "task_complete",
                            "turn_id": "turn-1",
                            "last_agent_message": "done",
                        },
                    },
                ], 1

            bridge.read_new_session_events = fake_read_new_session_events
            stdout = io.StringIO()
            with redirect_stdout(stdout):
                result = bridge.watch_for_final_answer(
                    Path("unused.jsonl"),
                    0,
                    timeout_sec=1,
                    include_commentary=True,
                    stream_live=True,
                )

            self.assertTrue(result["streamed_live"])
            self.assertTrue(result["final_streamed_live"])
            self.assertIn("[commentary]", stdout.getvalue())
            self.assertIn("[final_answer]", stdout.getvalue())
        finally:
            bridge.read_new_session_events = original_read_new_session_events

    async def test_mirror_session_target_sends_new_session_items_and_updates_cursor(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_resolve_target_ref = bot.resolve_target_ref
        try:
            updates: list[tuple[str, str, int]] = []
            claims: list[str] = []
            target_channel = FakeTarget(channel_id=333)
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("", encoding="utf-8")
                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=0,
                )

                def fake_read_new_session_events(path, cursor):
                    return _final_turn_events("mirrored done"), 42

                bridge.read_new_session_events = fake_read_new_session_events
                bot.get_or_init_session_mirror_cursor = lambda codex_thread_id, rollout_path, initial_cursor: 0
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )

                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: False

                def fake_claim_event(event_digest, codex_thread_id):
                    claims.append(event_digest)
                    return True

                bot.claim_session_mirror_event = fake_claim_event
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")
                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                await client.mirror_session_target(
                    {
                        "codex_thread_id": "thread-1",
                        "discord_thread_id": 333,
                        "discord_channel_id": 222,
                    }
                )
                await client.close()

            self.assertEqual(target_channel.messages, [("Final\n\nmirrored done", None)])
            self.assertEqual(len(claims), 1)
            self.assertEqual(updates, [("thread-1", str(session_path), 42)])
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_mirror_session_target_tails_archive_recommended_thread(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_context_usage = bridge.get_thread_context_usage
        original_should_recommend_archive = bridge.should_recommend_archive
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_resolve_target_ref = bot.resolve_target_ref
        try:
            updates: list[tuple[str, str, int]] = []
            read_calls: list[tuple[int, int | None]] = []
            target_channel = FakeTarget(channel_id=333)
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("old backlog\n", encoding="utf-8")
                log_path = Path(temp_dir) / "discord-smoke.log"

                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=999_999_999,
                )
                bridge.get_thread_context_usage = lambda thread: None
                bridge.should_recommend_archive = lambda thread, context_usage: True

                def fake_read_new_session_events(path, cursor, *, max_events=None):
                    read_calls.append((cursor, max_events))
                    return _final_turn_events("archived tail"), 42

                bridge.read_new_session_events = fake_read_new_session_events
                bot.get_or_init_session_mirror_cursor = lambda codex_thread_id, rollout_path, initial_cursor: 0
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )
                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: False
                bot.claim_session_mirror_event = lambda event_digest, codex_thread_id: True
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")

                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await client.mirror_session_target(
                        {
                            "codex_thread_id": "thread-1",
                            "discord_thread_id": 333,
                            "discord_channel_id": 222,
                        }
                    )
                await client.close()

                log_text = log_path.read_text(encoding="utf-8")
                self.assertIn("session_mirror_archive_tail_only target=thread-1 reason=archive_recommended", log_text)
                self.assertIn("session_mirror_archive_backlog_batch target=thread-1 events=2 max_events=200", log_text)
                self.assertEqual(target_channel.messages, [("Final\n\narchived tail", None)])
                self.assertEqual(read_calls, [(0, 200)])
                self.assertEqual(updates, [("thread-1", str(session_path), 42)])
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_context_usage = original_get_context_usage
            bridge.should_recommend_archive = original_should_recommend_archive
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_mirror_session_target_allows_archive_recommended_active_output(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_context_usage = bridge.get_thread_context_usage
        original_should_recommend_archive = bridge.should_recommend_archive
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_resolve_target_ref = bot.resolve_target_ref
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        try:
            updates: list[tuple[str, str, int]] = []
            target_channel = FakeTarget(channel_id=333)
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.activate_session_mirror_output_target("thread-1")
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("", encoding="utf-8")
                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=999_999_999,
                )
                bridge.get_thread_context_usage = lambda thread: None
                bridge.should_recommend_archive = lambda thread, context_usage: True

                def fake_read_new_session_events(path, cursor):
                    return _final_turn_events("active done"), 42

                bridge.read_new_session_events = fake_read_new_session_events
                bot.get_or_init_session_mirror_cursor = lambda codex_thread_id, rollout_path, initial_cursor: 0
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )
                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: False
                bot.claim_session_mirror_event = lambda event_digest, codex_thread_id: True
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")
                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                await client.mirror_session_target(
                    {
                        "codex_thread_id": "thread-1",
                        "discord_thread_id": 333,
                        "discord_channel_id": 222,
                    }
                )
                await client.close()

            self.assertEqual(target_channel.messages, [("Final\n\nactive done", None)])
            self.assertEqual(updates, [("thread-1", str(session_path), 42)])
            self.assertFalse(bot.is_active_session_mirror_output_target("thread-1"))
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_context_usage = original_get_context_usage
            bridge.should_recommend_archive = original_should_recommend_archive
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_mirror_session_target_does_not_claim_or_advance_cursor_when_delivery_fails(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_context_usage = bridge.get_thread_context_usage
        original_should_recommend_archive = bridge.should_recommend_archive
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_retry_delays = bot.DISCORD_SEND_RETRY_DELAYS_SECONDS
        original_resolve_target_ref = bot.resolve_target_ref
        try:
            updates: list[tuple[str, str, int]] = []
            claims: list[str] = []
            target_channel = AlwaysFailingTarget(channel_id=333)
            bot.DISCORD_SEND_RETRY_DELAYS_SECONDS = ()
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("", encoding="utf-8")
                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=0,
                )
                bridge.get_thread_context_usage = lambda thread: None
                bridge.should_recommend_archive = lambda thread, context_usage: False
                bridge.read_new_session_events = lambda path, cursor: (
                    _final_turn_events("must retry later"),
                    42,
                )
                bot.get_or_init_session_mirror_cursor = lambda codex_thread_id, rollout_path, initial_cursor: 0
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )
                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: False
                bot.claim_session_mirror_event = lambda event_digest, codex_thread_id: claims.append(event_digest)
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")
                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                with self.assertRaises(RuntimeError):
                    await client.mirror_session_target(
                        {
                            "codex_thread_id": "thread-1",
                            "discord_thread_id": 333,
                            "discord_channel_id": 222,
                        }
                    )
                await client.close()

            self.assertEqual(target_channel.messages, [])
            self.assertEqual(claims, [])
            self.assertEqual(updates, [])
        finally:
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_context_usage = original_get_context_usage
            bridge.should_recommend_archive = original_should_recommend_archive
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.DISCORD_SEND_RETRY_DELAYS_SECONDS = original_retry_delays
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_mirror_session_target_keeps_active_output_when_terminal_already_delivered(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_context_usage = bridge.get_thread_context_usage
        original_should_recommend_archive = bridge.should_recommend_archive
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_resolve_target_ref = bot.resolve_target_ref
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        try:
            updates: list[tuple[str, str, int]] = []
            target_channel = FakeTarget(channel_id=333)
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.activate_session_mirror_output_target("thread-1")
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("", encoding="utf-8")
                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=999_999_999,
                )
                bridge.get_thread_context_usage = lambda thread: None
                bridge.should_recommend_archive = lambda thread, context_usage: True
                bridge.read_new_session_events = lambda path, cursor: (
                    _final_turn_events("already claimed"),
                    42,
                )
                bot.get_or_init_session_mirror_cursor = lambda codex_thread_id, rollout_path, initial_cursor: 0
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )
                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: True
                bot.claim_session_mirror_event = lambda event_digest, codex_thread_id: (_ for _ in ()).throw(
                    AssertionError("already-seen mirror item should not be claimed again")
                )
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")
                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                await client.mirror_session_target(
                    {
                        "codex_thread_id": "thread-1",
                        "discord_thread_id": 333,
                        "discord_channel_id": 222,
                    }
                )
                await client.close()

            self.assertEqual(target_channel.messages, [])
            self.assertEqual(updates, [("thread-1", str(session_path), 42)])
            self.assertTrue(bot.is_active_session_mirror_output_target("thread-1"))
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_context_usage = original_get_context_usage
            bridge.should_recommend_archive = original_should_recommend_archive
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.resolve_target_ref = original_resolve_target_ref

    async def test_mirror_session_target_initializes_pending_active_cursor_from_start(self) -> None:
        original_choose_thread = bridge.choose_thread
        original_get_context_usage = bridge.get_thread_context_usage
        original_should_recommend_archive = bridge.should_recommend_archive
        original_read_new_session_events = bridge.read_new_session_events
        original_get_cursor = bot.get_or_init_session_mirror_cursor
        original_update_cursor = bot.update_session_mirror_cursor
        original_has_event = bot.has_session_mirror_event
        original_claim_event = bot.claim_session_mirror_event
        original_resolve_target_ref = bot.resolve_target_ref
        old_active_targets = dict(bot.get_session_mirror_state().active_output_targets)
        old_pending_targets = set(bot.get_session_mirror_state().pending_cursor_targets)
        try:
            cursor_calls: list[tuple[str, str, int]] = []
            updates: list[tuple[str, str, int]] = []
            read_cursors: list[int] = []
            target_channel = FakeTarget(channel_id=333)
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.clear()
            bot.activate_pending_session_mirror_output_target("thread-1")
            with tempfile.TemporaryDirectory() as temp_dir:
                session_path = Path(temp_dir) / "session.jsonl"
                session_path.write_text("new session content\n", encoding="utf-8")
                bridge.choose_thread = lambda thread_id, cwd=None: SimpleNamespace(
                    rollout_path=str(session_path),
                    tokens_used=999_999_999,
                )
                bridge.get_thread_context_usage = lambda thread: None
                bridge.should_recommend_archive = lambda thread, context_usage: True

                def fake_read_new_session_events(path, cursor):
                    read_cursors.append(cursor)
                    return _final_turn_events("pending done"), 42

                def fake_get_cursor(codex_thread_id, rollout_path, initial_cursor):
                    cursor_calls.append((codex_thread_id, rollout_path, initial_cursor))
                    return initial_cursor

                bridge.read_new_session_events = fake_read_new_session_events
                bot.get_or_init_session_mirror_cursor = fake_get_cursor
                bot.update_session_mirror_cursor = lambda codex_thread_id, rollout_path, cursor: updates.append(
                    (codex_thread_id, rollout_path, cursor)
                )
                bot.has_session_mirror_event = lambda event_digest, codex_thread_id: False
                bot.claim_session_mirror_event = lambda event_digest, codex_thread_id: True
                bot.resolve_target_ref = lambda target_thread_id: (target_thread_id, "project:1")
                log_path = Path(temp_dir) / "discord-smoke.log"
                client = bot.CodexDiscordBot(
                    allowed_channel_ids=set(),
                    allowed_user_ids=set(),
                    startup_channel_id=None,
                    guild_id=None,
                    enable_prefix_commands=True,
                )

                async def fake_resolve_channel(discord_thread_id):
                    return target_channel

                client.resolve_session_mirror_channel = fake_resolve_channel
                with EnvPatch("CODEX_DISCORD_LOG_PATH", str(log_path)):
                    await client.mirror_session_target(
                        {
                            "codex_thread_id": "thread-1",
                            "discord_thread_id": 333,
                            "discord_channel_id": 222,
                        }
                    )
                await client.close()
                log_text = log_path.read_text(encoding="utf-8")

            self.assertEqual(cursor_calls, [])
            self.assertEqual(read_cursors, [0])
            self.assertEqual(target_channel.messages, [("Final\n\npending done", None)])
            self.assertEqual(
                updates,
                [("thread-1", str(session_path), 0), ("thread-1", str(session_path), 42)],
            )
            self.assertFalse(bot.is_active_session_mirror_output_target("thread-1"))
            self.assertFalse(bot.is_pending_session_mirror_cursor_target("thread-1"))
            self.assertIn("session_mirror_pending_cursor_initialized target=thread-1 cursor=0", log_text)
        finally:
            bot.get_session_mirror_state().active_output_targets.clear()
            bot.get_session_mirror_state().active_output_targets.update(old_active_targets)
            bot.get_session_mirror_state().pending_cursor_targets.clear()
            bot.get_session_mirror_state().pending_cursor_targets.update(old_pending_targets)
            bridge.choose_thread = original_choose_thread
            bridge.get_thread_context_usage = original_get_context_usage
            bridge.should_recommend_archive = original_should_recommend_archive
            bridge.read_new_session_events = original_read_new_session_events
            bot.get_or_init_session_mirror_cursor = original_get_cursor
            bot.update_session_mirror_cursor = original_update_cursor
            bot.has_session_mirror_event = original_has_event
            bot.claim_session_mirror_event = original_claim_event
            bot.resolve_target_ref = original_resolve_target_ref


if __name__ == "__main__":
    unittest.main()
