from __future__ import annotations

from builtins import BaseExceptionGroup
from collections.abc import Callable
from contextlib import nullcontext
from pathlib import Path
from typing import Protocol, cast, final

import pytest

import codex_discord_cli
import codex_discord_bot_main_runtime
from codex_discord_bot_main_runtime import BotRunner, RuntimeCloser


class _RunBotAndCloseServices(Protocol):
    def __call__(
        self,
        bot: BotRunner,
        token: str,
        *,
        close_bridge: RuntimeCloser,
        close_app_server: RuntimeCloser,
    ) -> None: ...


_run_bot_and_close_services = cast(
    _RunBotAndCloseServices,
    cast(
        object, getattr(codex_discord_bot_main_runtime, "_run_bot_and_close_services")
    ),
)


@final
class _RecordingRunner:
    def __init__(
        self,
        events: list[str],
        error: BaseException | None = None,
    ) -> None:
        self._events = events
        self._error = error

    def run(self, token: str, *, log_handler: None) -> None:
        assert token == "token"
        assert log_handler is None
        self._events.append("run")
        if self._error is not None:
            raise self._error


@final
class _RecordingBotFactory:
    def __init__(self, events: list[str]) -> None:
        self._events = events

    def __call__(
        self,
        *,
        allowed_channel_ids: set[int],
        allowed_user_ids: set[int],
        startup_channel_id: int | None,
        guild_id: int | None,
        enable_prefix_commands: bool,
        plain_ask_mention_user_ids: set[int],
    ) -> _RecordingRunner:
        _ = (
            allowed_channel_ids,
            allowed_user_ids,
            startup_channel_id,
            guild_id,
            enable_prefix_commands,
            plain_ask_mention_user_ids,
        )
        return _RecordingRunner(self._events)


@final
class _ParsedArgs:
    no_message_content = False


@final
class _Parser:
    @staticmethod
    def parse_args() -> _ParsedArgs:
        return _ParsedArgs()


def _closer(
    events: list[str],
    name: str,
    error: BaseException | None = None,
) -> Callable[[], None]:
    def close() -> None:
        events.append(name)
        if error is not None:
            raise error

    return close


def test_main_connects_remote_device_before_bot_run_after_restart(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    # Given
    events: list[str] = []
    logs: list[str] = []
    monkeypatch.setenv("DISCORD_BOT_TOKEN", "token")
    monkeypatch.setenv("DISCORD_ALLOWED_CHANNEL_IDS", "1")
    monkeypatch.delenv("DISCORD_ALLOW_ALL_CHANNELS", raising=False)
    monkeypatch.delenv("CODEX_REMOTE_MCP_ENABLED", raising=False)
    monkeypatch.setattr(
        codex_discord_cli,
        "build_parser",
        _Parser,
    )
    monkeypatch.setattr(codex_discord_bot_main_runtime, "log_line", logs.append)

    def connect_device(root: Path, log: Callable[[str], None]) -> None:
        _ = log
        assert root == tmp_path
        events.append("connect")

    monkeypatch.setattr(
        codex_discord_bot_main_runtime,
        "connect_remote_mcp_device",
        connect_device,
    )
    deps = codex_discord_bot_main_runtime.BotMainDeps(
        env_path=tmp_path / ".env",
        bot_factory=_RecordingBotFactory(events),
        acquire_runtime_instance_lock=lambda: nullcontext(True),
    )

    # When
    exit_code = codex_discord_bot_main_runtime.main(deps)

    # Then
    assert exit_code == 0
    assert events == ["connect", "run"]


def test_bot_failure_and_bridge_cleanup_failure_still_close_app_server() -> None:
    events: list[str] = []
    runner = _RecordingRunner(events, RuntimeError("bot failed"))

    with pytest.raises(BaseExceptionGroup) as captured:
        _run_bot_and_close_services(
            runner,
            "token",
            close_bridge=_closer(events, "bridge", ValueError("bridge failed")),
            close_app_server=_closer(events, "app-server"),
        )

    assert events == ["run", "bridge", "app-server"]
    assert tuple(type(error) for error in captured.value.exceptions) == (
        RuntimeError,
        ValueError,
    )


def test_every_cleanup_failure_is_reported_after_a_normal_bot_exit() -> None:
    events: list[str] = []
    runner = _RecordingRunner(events)

    with pytest.raises(BaseExceptionGroup) as captured:
        _run_bot_and_close_services(
            runner,
            "token",
            close_bridge=_closer(events, "bridge", ValueError("bridge failed")),
            close_app_server=_closer(
                events,
                "app-server",
                OSError("app server failed"),
            ),
        )

    assert events == ["run", "bridge", "app-server"]
    assert tuple(type(error) for error in captured.value.exceptions) == (
        ValueError,
        OSError,
    )
