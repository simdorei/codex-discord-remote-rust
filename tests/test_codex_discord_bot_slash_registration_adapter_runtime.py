from __future__ import annotations

from collections.abc import Callable
import unittest
from types import ModuleType
from typing import final
from unittest.mock import patch

import codex_discord_bot_slash_registration_adapter_runtime as adapter_runtime
import codex_discord_slash_registration as slash_registration
import codex_discord_slash_types as slash_types


@final
class FakeSlashCommandTree:
    def command(
        self,
        *,
        name: str,
        description: str,
    ) -> Callable[[slash_types.SlashCallback], slash_types.SlashCallback]:
        _ = (name, description)

        def decorate(
            callback: slash_types.SlashCallback,
        ) -> slash_types.SlashCallback:
            return callback

        return decorate


@final
class FakeSlashCommandBot:
    def __init__(self) -> None:
        self._tree: FakeSlashCommandTree = FakeSlashCommandTree()

    @property
    def tree(self) -> slash_types.SlashCommandTree:
        return self._tree


class FakeRuntimeConfig:
    def discord_qa_commands_enabled(self) -> bool:
        return False

    def discord_host_commands_enabled(self) -> bool:
        return False


class FakeAppServerClient:
    def __init__(self) -> None:
        self.last_request: tuple[str, dict[str, str], float] | None = None
        self.request_count: int = 0

    def request(
        self,
        method: str,
        params: dict[str, str],
        *,
        timeout_sec: float,
    ) -> dict[str, list[dict[str, str | bool]]]:
        self.request_count += 1
        self.last_request = (method, params, timeout_sec)
        return {"data": [{"model": "gpt-5.6-terra", "hidden": False}]}


class FakeAppServerTransport:
    DEFAULT_CLIENT: FakeAppServerClient = FakeAppServerClient()


@final
class FakeSlashRegistrationModule(ModuleType):
    def __init__(self) -> None:
        super().__init__("fake_slash_registration_adapter_module")
        self.discord_runtime_config: FakeRuntimeConfig = FakeRuntimeConfig()
        self.app_server_transport: FakeAppServerTransport = FakeAppServerTransport()


class SlashRegistrationAdapterRuntimeTests(unittest.TestCase):
    def test_register_commands_builds_deps_without_runtime_generic_error(self) -> None:
        module = self.make_module()
        adapter = adapter_runtime.BotSlashRegistrationAdapterRuntime(module=module)
        FakeAppServerTransport.DEFAULT_CLIENT.request_count = 0
        captured: list[slash_registration.SlashRegistrationDeps[object, object]] = []

        def fake_register_commands(
            bot: object,
            deps: slash_registration.SlashRegistrationDeps[object, object],
        ) -> None:
            _ = bot
            captured.append(deps)

        with patch.object(slash_registration, "register_commands", fake_register_commands):
            adapter.register_commands(FakeSlashCommandBot())

        self.assertEqual(len(captured), 1)
        self.assertEqual(captured[0].build_help().splitlines()[0], "Codex Discord commands")
        self.assertEqual(
            captured[0].load_settings_model_catalog(),
            {"data": [{"model": "gpt-5.6-terra", "hidden": False}]},
        )
        self.assertEqual(
            captured[0].load_settings_model_catalog(),
            {"data": [{"model": "gpt-5.6-terra", "hidden": False}]},
        )
        self.assertEqual(FakeAppServerTransport.DEFAULT_CLIENT.request_count, 1)
        self.assertEqual(
            FakeAppServerTransport.DEFAULT_CLIENT.last_request,
            ("model/list", {}, 2.0),
        )

    def make_module(self) -> ModuleType:
        module = FakeSlashRegistrationModule()

        def check_interaction_allowed(bot: object, interaction: object) -> bool:
            _ = (bot, interaction)
            return True

        def require_discord_interaction(interaction: object) -> object:
            return interaction

        async def send_interaction_not_allowed(interaction: object) -> None:
            _ = interaction

        async def send_interaction_chunks(interaction: object, text: str, *, title: str) -> None:
            _ = (interaction, text, title)

        async def run_interaction_bridge_and_send(
            interaction: object,
            argv: list[str],
            title: str,
        ) -> tuple[int, str]:
            _ = (interaction, argv, title)
            return 0, ""

        async def send_interaction_response_tracked(
            interaction: object,
            content: str,
            *,
            ephemeral: bool = False,
            context: str = "interaction_response",
        ) -> None:
            _ = (interaction, content, ephemeral, context)

        async def build_runtime_discord_doctor_message(
            bot: object,
            channel_id: int | None,
            channel: object | None,
        ) -> str:
            _ = (bot, channel_id, channel)
            return "doctor"

        async def build_runners_message() -> str:
            return "runners"

        async def retract_queued_ask_for_request(
            channel_id: int | None,
            user_id: int | None,
            request_id: str | None,
        ) -> str:
            _ = (channel_id, user_id, request_id)
            return "retracted"

        async def refresh_runtime_discord_bridge_session(channel_id: int | None) -> str:
            _ = channel_id
            return "refreshed"

        async def run_runtime_discord_button_qa(
            bot: object,
            channel_id: int,
            user_id: int,
        ) -> str:
            _ = (bot, channel_id, user_id)
            return "qa"

        async def handle_slash_new(bot: object, interaction: object, prompt: str) -> None:
            _ = (bot, interaction, prompt)

        async def handle_slash_prompt(interaction: object, prompt: str) -> None:
            _ = (interaction, prompt)

        funcs: dict[str, object] = {
            "check_interaction_allowed": check_interaction_allowed,
            "require_discord_interaction": require_discord_interaction,
            "send_interaction_not_allowed": send_interaction_not_allowed,
            "send_interaction_chunks": send_interaction_chunks,
            "run_interaction_bridge_and_send": run_interaction_bridge_and_send,
            "send_interaction_response_tracked": send_interaction_response_tracked,
            "build_where_message": lambda channel_id: f"where:{channel_id}",
            "build_context_message": lambda channel_id, target=None, limit=None: "context",
            "build_context_refresh_message": lambda channel_id, target=None, limit=None: "refresh",
            "build_weekly_usage_message": lambda channel_id=None: "usage",
            "clamp_context_refresh_limit": lambda limit: limit,
            "resolve_discord_thread_target_args": lambda channel_id, target: [],
            "build_mirror_check": lambda: "mirror",
            "build_runtime_discord_doctor_message": build_runtime_discord_doctor_message,
            "build_runners_message": build_runners_message,
            "retract_queued_ask_for_request": retract_queued_ask_for_request,
            "refresh_runtime_discord_bridge_session": refresh_runtime_discord_bridge_session,
            "run_runtime_discord_button_qa": run_runtime_discord_button_qa,
            "handle_slash_new": handle_slash_new,
            "handle_slash_ask": handle_slash_prompt,
            "handle_slash_interview": handle_slash_prompt,
            "log_line": lambda message: None,
        }
        for name, value in funcs.items():
            setattr(module, name, value)
        self.assertFalse(hasattr(module, "handle_slash_github_triage"))
        self.assertFalse(hasattr(module, "handle_slash_maintainer_orchestrator"))
        return module


if __name__ == "__main__":
    unittest.main()
