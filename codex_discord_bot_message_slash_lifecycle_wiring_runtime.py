from __future__ import annotations

from dataclasses import dataclass
from types import ModuleType
from typing import cast

import codex_discord_bot_lifecycle_adapter_runtime as discord_bot_lifecycle_adapter_runtime
from codex_discord_bot_module_context import install_runtime_bindings
import codex_discord_bot_message_adapter_runtime as discord_bot_message_adapter_runtime
import codex_discord_bot_misc_adapter_runtime as discord_bot_misc_adapter_runtime
import codex_discord_bot_slash_registration_adapter_runtime as discord_bot_slash_registration_adapter_runtime


@dataclass(frozen=True, slots=True)
class BotMessageSlashLifecycleWiringRuntime:
    module: ModuleType

    def install(self) -> None:
        self._install_message_runtime()
        self._install_slash_registration_runtime()
        self._install_lifecycle_runtime()
        self._install_misc_exports()

    def _install_message_runtime(self) -> None:
        adapter_runtime = discord_bot_message_adapter_runtime.BotMessageAdapterRuntime(
            module=self.module,
        )
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={
                "MESSAGE_ADAPTER_RUNTIME": adapter_runtime,
                "handle_prefix_command": adapter_runtime.handle_prefix_command,
                "MESSAGE_RUNTIME": adapter_runtime.make_message_runtime(),
            },
        )

    def _install_slash_registration_runtime(self) -> None:
        adapter_runtime = (
            discord_bot_slash_registration_adapter_runtime.BotSlashRegistrationAdapterRuntime(
                module=self.module,
            )
        )
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={
                "SLASH_REGISTRATION_ADAPTER_RUNTIME": adapter_runtime,
                "build_help": adapter_runtime.build_help,
                "register_commands": adapter_runtime.register_commands,
            },
        )

    def _install_lifecycle_runtime(self) -> None:
        adapter_runtime = discord_bot_lifecycle_adapter_runtime.BotLifecycleAdapterRuntime(
            module=self.module,
        )
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={
                "LIFECYCLE_ADAPTER_RUNTIME": adapter_runtime,
                "LIFECYCLE_RUNTIME": adapter_runtime.make_lifecycle_runtime(),
            },
        )

    def _install_misc_exports(self) -> None:
        misc_adapter_runtime = cast(
            discord_bot_misc_adapter_runtime.BotMiscAdapterRuntime,
            getattr(self.module, "MISC_ADAPTER_RUNTIME"),
        )
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={
                "check_interaction_allowed": misc_adapter_runtime.check_interaction_allowed,
                "main": misc_adapter_runtime.main,
            },
        )
