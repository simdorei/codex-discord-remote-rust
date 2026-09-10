from __future__ import annotations

from builtins import BaseExceptionGroup
from collections.abc import Callable
from contextlib import AbstractContextManager
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import codex_app_server_transport as app_server_transport
import codex_discord_cli as discord_cli
import codex_discord_message_gate as discord_message_gate
import codex_discord_runtime_config as discord_runtime_config
from codex_discord_logging import log_line
from codex_remote_mcp_binding import (
    close_remote_mcp_bridge,
    connect_remote_mcp_device,
    restore_remote_mcp_restart_handoff,
)
from codex_remote_mcp_bridge_config import RemoteMcpConfigurationError
from codex_remote_mcp_bridge_connection import RemoteMcpBridgeError
from codex_remote_mcp_restart_models import RestartHandoffError


class BotRunner(Protocol):
    def run(self, token: str, *, log_handler: None) -> None: ...


class BotFactory(Protocol):
    def __call__(
        self,
        *,
        allowed_channel_ids: set[int],
        allowed_user_ids: set[int],
        startup_channel_id: int | None,
        guild_id: int | None,
        enable_prefix_commands: bool,
        plain_ask_mention_user_ids: set[int],
    ) -> BotRunner: ...


RuntimeCloser = Callable[[], None]


@dataclass(frozen=True, slots=True)
class BotMainDeps:
    env_path: Path
    bot_factory: BotFactory
    acquire_runtime_instance_lock: Callable[[], AbstractContextManager[bool]]


def main(deps: BotMainDeps) -> int:
    discord_runtime_config.load_local_env(deps.env_path)
    args = discord_cli.build_parser().parse_args()
    token = discord_runtime_config.get_required_env("DISCORD_BOT_TOKEN")
    channel_ids = discord_runtime_config.get_discord_allowed_channel_ids()
    user_ids = discord_runtime_config.get_discord_allowed_user_ids()
    plain_ask_mention_user_ids = discord_runtime_config.get_plain_ask_mention_user_ids()
    allow_all_channels = discord_runtime_config.discord_allow_all_channels_enabled()
    if not channel_ids and not allow_all_channels:
        log_line("main_config_error reason=missing_allowed_channels")
        print("ERROR: Set DISCORD_ALLOWED_CHANNEL_IDS or DISCORD_ALLOW_ALL_CHANNELS=1.")
        return 1
    startup_channel_id = discord_runtime_config.get_startup_channel_id(channel_ids)
    guild_id = discord_runtime_config.get_discord_guild_id()
    no_message_content = bool(getattr(args, "no_message_content", False))
    enable_prefix_commands = (
        discord_runtime_config.discord_message_content_enabled()
        and not no_message_content
    )
    with deps.acquire_runtime_instance_lock() as runtime_lock_acquired:
        if not runtime_lock_acquired:
            print("ERROR: Codex Discord bot is already running.")
            return 2
        try:
            _ = restore_remote_mcp_restart_handoff(log_line)
        except (OSError, RestartHandoffError, RemoteMcpBridgeError) as exc:
            log_line(
                "remote_mcp_restart_handoff_restore_failed "
                + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
            )
        try:
            _ = connect_remote_mcp_device(deps.env_path.parent, log_line)
        except (RemoteMcpConfigurationError, RemoteMcpBridgeError) as exc:
            log_line(
                "remote_mcp_device_startup_connect_failed "
                + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
            )
        bot = deps.bot_factory(
            allowed_channel_ids=channel_ids,
            allowed_user_ids=user_ids,
            startup_channel_id=startup_channel_id,
            guild_id=guild_id,
            enable_prefix_commands=enable_prefix_commands,
            plain_ask_mention_user_ids=plain_ask_mention_user_ids,
        )
        log_line(
            " ".join(
                (
                    "main_start",
                    f"guild_id={guild_id or '-'}",
                    f"channels={sorted(channel_ids) if channel_ids else 'ALL_EXPLICIT'}",
                    f"users={sorted(user_ids) if user_ids else 'ALL'}",
                    f"message_content={enable_prefix_commands}",
                    f"plain_ask_mentions={sorted(plain_ask_mention_user_ids) if plain_ask_mention_user_ids else '-'}",
                    f"plain_ask_context_fallback={discord_message_gate.plain_ask_context_fallback_enabled()}",
                    f"qa_commands={discord_runtime_config.discord_qa_commands_enabled()}",
                )
            )
        )
        _run_bot_and_close_services(
            bot,
            token,
            close_bridge=close_remote_mcp_bridge,
            close_app_server=app_server_transport.DEFAULT_CLIENT.close,
        )
    return 0


def _run_bot_and_close_services(
    bot: BotRunner,
    token: str,
    *,
    close_bridge: RuntimeCloser,
    close_app_server: RuntimeCloser,
) -> None:
    try:
        bot.run(token, log_handler=None)
    except BaseException as run_error:  # noqa: BLE001 - cleanup must include shutdowns.
        cleanup_errors = _close_runtime_services(close_bridge, close_app_server)
        if cleanup_errors:
            raise BaseExceptionGroup(
                "Discord bot runtime and cleanup failures",
                (run_error, *cleanup_errors),
            ) from None
        raise
    cleanup_errors = _close_runtime_services(close_bridge, close_app_server)
    if len(cleanup_errors) == 1:
        raise cleanup_errors[0]
    if cleanup_errors:
        raise BaseExceptionGroup(
            "Discord bot cleanup failures",
            cleanup_errors,
        )


def _close_runtime_services(
    close_bridge: RuntimeCloser,
    close_app_server: RuntimeCloser,
) -> tuple[BaseException, ...]:
    errors: list[BaseException] = []
    for close_service in (close_bridge, close_app_server):
        try:
            close_service()
        except BaseException as exc:  # noqa: BLE001 - report every cleanup failure.
            errors.append(exc)
    return tuple(errors)
