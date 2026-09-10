from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, cast, TypeAlias

import codex_app_server_transport as app_server_transport
import codex_discord_bot_lifecycle_runtime as discord_bot_lifecycle_runtime
import codex_discord_interaction_log as discord_interaction_log
import codex_discord_ready_cleanup as discord_ready_cleanup
import codex_discord_startup_notify as discord_startup_notify
ModuleValue: TypeAlias = object


class DiscordObjectFactory(Protocol):
    def __call__(self, *, id: int) -> ModuleValue: ...


class DiscordAbcModule(Protocol):
    Messageable: type[ModuleValue]


class DiscordModule(Protocol):
    Object: DiscordObjectFactory
    abc: DiscordAbcModule


@dataclass(frozen=True, slots=True)
class BotLifecycleAdapterRuntime:
    module: ModuleType

    def make_lifecycle_runtime(self) -> discord_bot_lifecycle_runtime.BotLifecycleRuntime[ModuleValue]:
        return discord_bot_lifecycle_runtime.BotLifecycleRuntime(
            discord_bot_lifecycle_runtime.BotLifecycleRuntimeDeps(
                app_server_transport_enabled=self.app_server_transport_enabled,
                start_app_server_transport=self.start_app_server_transport,
                run_in_thread=lambda func: asyncio.to_thread(func),
                register_commands=self.register_commands,
                make_guild_object=self.make_guild_object,
                wait_for_slash_sync=self.wait_for_slash_sync,
                run_ready_maintenance=self.run_ready_maintenance,
                restore_queue_runners=self.restore_queue_runners,
                send_startup_notice=self.send_startup_notice,
                format_user_id=discord_interaction_log.format_discord_user_id_for_log,
                delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
                log=cast(Callable[[str], None], self._module_func("log_line")),
            )
        )

    def app_server_transport_enabled(self) -> bool:
        return cast(Callable[[], bool], self._module_func("app_server_transport_enabled"))()

    def start_app_server_transport(self) -> None:
        app_server_transport.DEFAULT_CLIENT.start()

    def register_commands(self, bot: discord_bot_lifecycle_runtime.SetupHookBot[ModuleValue]) -> None:
        cast(Callable[[discord_bot_lifecycle_runtime.SetupHookBot[object]], None], self._module_func("register_commands"))(
            bot
        )

    def make_guild_object(self, guild_id: int) -> ModuleValue:
        discord_module = cast(DiscordModule, getattr(self.module, "discord"))
        return discord_module.Object(id=guild_id)

    async def wait_for_slash_sync(
        self,
        awaitable: Awaitable[Sequence[discord_bot_lifecycle_runtime.SlashCommand]],
        timeout: float,
    ) -> Sequence[discord_bot_lifecycle_runtime.SlashCommand]:
        return await asyncio.wait_for(awaitable, timeout=timeout)

    async def run_ready_maintenance(self, bot: discord_bot_lifecycle_runtime.ReadyBot) -> None:
        await discord_ready_cleanup.run_ready_maintenance(
            cast(
                Callable[[discord_bot_lifecycle_runtime.ReadyBot], discord_ready_cleanup.ReadyMaintenanceDeps],
                self._module_func("_make_ready_maintenance_deps"),
            )(bot)
        )

    async def restore_queue_runners(self, bot: discord_bot_lifecycle_runtime.ReadyBot) -> int:
        return await cast(
            Callable[[object], Awaitable[int]],
            self._module_func("restore_durable_queue_runners"),
        )(bot)

    async def send_startup_notice(self, bot: discord_bot_lifecycle_runtime.ReadyBot) -> None:
        runtime_config = cast(object, getattr(self.module, "discord_runtime_config"))
        await discord_startup_notify.send_startup_notice_if_enabled(
            cast(discord_startup_notify.StartupNotifyClient[object], cast(object, bot)),
            bot.startup_channel_id,
            notify_enabled=cast(Callable[[], bool], getattr(runtime_config, "discord_startup_notify_enabled")),
            is_messageable=self.is_messageable,
            send_chunks=self.send_chunks,
            build_startup_notice=cast(Callable[[], str], self._module_func("build_startup_notice")),
            log=cast(Callable[[str], None], self._module_func("log_line")),
            delivery_exceptions=cast(tuple[type[BaseException], ...], getattr(self.module, "DISCORD_DELIVERY_EXCEPTIONS")),
        )

    def is_messageable(self, channel: ModuleValue) -> bool:
        discord_module = cast(DiscordModule, getattr(self.module, "discord"))
        return isinstance(channel, discord_module.abc.Messageable)

    async def send_chunks(self, channel: ModuleValue, text: str, *, context: str) -> int:
        return await cast(
            discord_startup_notify.StartupNoticeSender[object],
            self._module_func("send_chunks"),
        )(channel, text, context=context)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))
