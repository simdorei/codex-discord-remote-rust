from __future__ import annotations

from collections.abc import Awaitable, Callable, Sequence, Sized
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Generic, Protocol, TypeVar, cast

import codex_discord_interaction_log as discord_interaction_log


GuildObjectT = TypeVar("GuildObjectT")
GuildObjectContraT = TypeVar("GuildObjectContraT", contravariant=True)


class SlashCommand(Protocol):
    @property
    def name(self) -> str: ...


class SlashCommandTree(Protocol[GuildObjectContraT]):
    def copy_global_to(self, *, guild: GuildObjectContraT) -> None: ...

    def sync(self, guild: GuildObjectContraT | None = None) -> Awaitable[Sequence[SlashCommand]]: ...


class SetupHookBot(Protocol[GuildObjectContraT]):
    guild_id: int | None
    tree: SlashCommandTree[GuildObjectContraT]


class ReadyBot(Protocol):
    user: discord_interaction_log.DiscordUserLogValue
    guilds: Sized
    startup_channel_id: int | None

    def log_startup_diagnostics(self) -> Awaitable[None]: ...


class AsyncReadyHook(Protocol):
    def __call__(self) -> Awaitable[None]: ...


@dataclass(frozen=True, slots=True)
class BotLifecycleRuntimeDeps(Generic[GuildObjectT]):
    app_server_transport_enabled: Callable[[], bool]
    start_app_server_transport: Callable[[], None]
    run_in_thread: Callable[[Callable[[], None]], Awaitable[None]]
    register_commands: Callable[[SetupHookBot[GuildObjectT]], None]
    make_guild_object: Callable[[int], GuildObjectT]
    wait_for_slash_sync: Callable[
        [Awaitable[Sequence[SlashCommand]], float],
        Awaitable[Sequence[SlashCommand]],
    ]
    run_ready_maintenance: Callable[[ReadyBot], Awaitable[None]]
    restore_queue_runners: Callable[[ReadyBot], Awaitable[int]]
    send_startup_notice: Callable[[ReadyBot], Awaitable[None]]
    format_user_id: Callable[[discord_interaction_log.DiscordUserLogValue], str]
    delivery_exceptions: tuple[type[BaseException], ...]
    log: Callable[[str], None]


@dataclass(frozen=True, slots=True)
class BotLifecycleRuntime(Generic[GuildObjectT]):
    deps: BotLifecycleRuntimeDeps[GuildObjectT]

    async def setup_hook(self, bot: SetupHookBot[GuildObjectT]) -> None:
        self.deps.log("setup_hook_start")
        if self.deps.app_server_transport_enabled():
            try:
                await self.deps.run_in_thread(self.deps.start_app_server_transport)
                self.deps.log("setup_hook_app_server_transport_ready")
            except self.deps.delivery_exceptions as exc:
                self.deps.log(f"setup_hook_app_server_transport_failed error={str(exc)[:300]}")
        self.deps.register_commands(bot)
        try:
            if bot.guild_id:
                guild = self.deps.make_guild_object(bot.guild_id)
                bot.tree.copy_global_to(guild=guild)
                self.deps.log(f"setup_hook_sync_guild guild_id={bot.guild_id}")
                synced = await self.deps.wait_for_slash_sync(bot.tree.sync(guild=guild), 20.0)
            else:
                self.deps.log("setup_hook_sync_global")
                synced = await self.deps.wait_for_slash_sync(bot.tree.sync(), 20.0)
            command_names = sorted(command.name for command in synced)
            self._set_slash_sync_state(bot, status="ok", commands=",".join(command_names) or "-")
            self.deps.log(f"setup_hook_synced commands={','.join(command_names) or '-'}")
        except self.deps.delivery_exceptions as exc:
            self._set_slash_sync_state(bot, status=f"skipped:{type(exc).__name__}", commands="-")
            self.deps.log(f"setup_hook_sync_skipped error={exc}")
        self.deps.log("setup_hook_done")

    async def on_ready(self, bot: ReadyBot) -> None:
        self.deps.log(
            " ".join(
                (
                    f"ready user_id={self.deps.format_user_id(bot.user)}",
                    f"guilds={len(bot.guilds)}",
                )
            )
        )
        await self.deps.run_ready_maintenance(bot)
        restored_jobs = await self.deps.restore_queue_runners(bot)
        self.deps.log(f"ready_queue_restore jobs={restored_jobs}")
        await self._call_optional_ready_hook(bot, "start_stop_marker_watcher")
        await self._call_optional_ready_hook(bot, "start_history_polling")
        await self.deps.send_startup_notice(bot)
        await self._call_optional_ready_hook(bot, "start_session_mirroring")
        await bot.log_startup_diagnostics()

    async def _call_optional_ready_hook(self, bot: ReadyBot, name: str) -> None:
        hook = getattr(bot, name, None)
        if not callable(hook):
            return
        await cast(AsyncReadyHook, hook)()

    def _set_slash_sync_state(
        self,
        bot: SetupHookBot[GuildObjectT],
        *,
        status: str,
        commands: str,
    ) -> None:
        setattr(bot, "_slash_sync_last_at", datetime.now(timezone.utc).isoformat(timespec="seconds"))
        setattr(bot, "_slash_sync_status", status)
        setattr(bot, "_slash_sync_commands", commands)
