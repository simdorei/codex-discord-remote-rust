from __future__ import annotations

import unittest
from collections.abc import Awaitable, Callable
from dataclasses import dataclass

import discord
from discord import app_commands

import codex_discord_slash_commands as slash_commands

SlashFunc = slash_commands.SlashCallback
AutocompleteFunc = Callable[..., Awaitable[list[app_commands.Choice[str]]]]


class FakeTree:
    def __init__(self) -> None:
        self.commands: dict[str, SlashFunc] = {}
        self.descriptions: dict[str, str] = {}
        self.autocomplete: dict[str, dict[str, AutocompleteFunc]] = {}

    def command(self, *, name: str, description: str) -> Callable[[SlashFunc], SlashFunc]:
        def decorate(func: SlashFunc) -> SlashFunc:
            self.commands[name] = func
            self.descriptions[name] = description
            callbacks = getattr(func, "__discord_app_commands_param_autocomplete__", {})
            self.autocomplete[name] = dict(callbacks)
            return func

        return decorate


class FakeBot:
    def __init__(self) -> None:
        self._tree: FakeTree = FakeTree()

    @property
    def tree(self) -> FakeTree:
        return self._tree


class RealTreeAdapter:
    def __init__(self, tree: app_commands.CommandTree[discord.Client]) -> None:
        self._tree = tree

    def command(self, *, name: str, description: str) -> Callable[[SlashFunc], SlashFunc]:
        def decorate(func: SlashFunc) -> SlashFunc:
            _ = self._tree.command(name=name, description=description)(func)
            return func

        return decorate


class RealTreeBot:
    def __init__(self) -> None:
        self.client: discord.Client = discord.Client(intents=discord.Intents.none())
        self.command_tree: app_commands.CommandTree[discord.Client] = app_commands.CommandTree(
            self.client
        )
        self._tree = RealTreeAdapter(self.command_tree)

    @property
    def tree(self) -> RealTreeAdapter:
        return self._tree


class FakeResponse:
    def __init__(self) -> None:
        self.deferred: bool = False
        self.defer_kwargs: list[dict[str, bool]] = []

    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        self.deferred = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})


@dataclass(frozen=True, slots=True)
class FakeNamespace:
    model: str


class FakeInteraction:
    def __init__(self, channel_id: int = 222, *, model: str = "") -> None:
        self.channel_id: int = channel_id
        self.response: FakeResponse = FakeResponse()
        self.namespace = FakeNamespace(model=model)


class BasicSlashCommandRegistrationTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        allowed: bool = True,
        run_calls: list[tuple[list[str], str]] | None = None,
        chunk_calls: list[tuple[str, str]] | None = None,
        denied_calls: list[slash_commands.SlashInteraction] | None = None,
        model_catalog: slash_commands.SettingsModelCatalog | None = None,
        model_catalog_loader: slash_commands.SettingsModelCatalogLoader | None = None,
    ) -> slash_commands.BasicSlashCommandDeps:
        async def send_not_allowed(interaction: slash_commands.SlashInteraction) -> None:
            if denied_calls is not None:
                denied_calls.append(interaction)

        async def send_chunks(
            interaction: slash_commands.SlashInteraction,
            text: str,
            *,
            title: str,
        ) -> None:
            _ = interaction
            if chunk_calls is not None:
                chunk_calls.append((title, text))

        async def run_bridge(
            interaction: slash_commands.SlashInteraction,
            argv: list[str],
            title: str,
        ) -> tuple[int, str]:
            _ = interaction
            if run_calls is not None:
                run_calls.append((argv, title))
            return 0, "ok"

        def build_context(
            channel_id: int | None,
            *,
            all_threads: bool = False,
            limit: int = 20,
        ) -> str:
            return f"context:{channel_id}:{all_threads}:{limit}"

        def build_context_refresh(channel_id: int | None, *, limit: int) -> str:
            return f"refresh:{channel_id}:{limit}"

        def build_weekly_usage(*, days: int) -> str:
            return f"usage:{days}"

        return slash_commands.BasicSlashCommandDeps(
            check_allowed=lambda interaction: allowed,
            send_not_allowed=send_not_allowed,
            send_chunks=send_chunks,
            run_bridge=run_bridge,
            build_help=lambda: "help text",
            build_where=lambda channel_id: f"where:{channel_id}",
            build_context=build_context,
            build_context_refresh=build_context_refresh,
            build_weekly_usage=build_weekly_usage,
            clamp_context_refresh_limit=lambda limit: max(1, min(30, int(limit))),
            resolve_target_args=lambda channel_id, ref: ["--target", f"{channel_id}:{ref or '-'}"],
            load_settings_model_catalog=model_catalog_loader
            or (lambda: model_catalog or {"data": []}),
        )

    def test_registers_basic_slash_command_names(self) -> None:
        bot = FakeBot()
        slash_commands.register_basic_slash_commands(bot, self.make_deps())

        self.assertEqual(
            set(bot.tree.commands),
            {
                "archived_list",
                "context",
                "help",
                "list",
                "settings",
                "status",
                "usage",
                "use",
                "where",
            },
        )

    async def test_denied_interaction_sends_not_allowed_without_defer(self) -> None:
        bot = FakeBot()
        denied_calls: list[slash_commands.SlashInteraction] = []
        slash_commands.register_basic_slash_commands(
            bot,
            self.make_deps(allowed=False, denied_calls=denied_calls),
        )
        interaction = FakeInteraction()

        await bot.tree.commands["help"](interaction)

        self.assertEqual(denied_calls, [interaction])
        self.assertFalse(interaction.response.deferred)

    async def test_status_runs_bridge_with_resolved_target_args(self) -> None:
        bot = FakeBot()
        run_calls: list[tuple[list[str], str]] = []
        slash_commands.register_basic_slash_commands(bot, self.make_deps(run_calls=run_calls))
        interaction = FakeInteraction(channel_id=333)

        await bot.tree.commands["status"](interaction, "repo:2")

        self.assertEqual(run_calls, [(["status", "--target", "333:repo:2"], "Status")])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])

    async def test_context_refresh_uses_clamped_limit(self) -> None:
        bot = FakeBot()
        chunk_calls: list[tuple[str, str]] = []
        slash_commands.register_basic_slash_commands(bot, self.make_deps(chunk_calls=chunk_calls))
        interaction = FakeInteraction(channel_id=444)

        await bot.tree.commands["context"](interaction, refresh=True, limit=77)

        self.assertEqual(chunk_calls, [("Context", "refresh:444:30")])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])

    async def test_usage_clamps_days(self) -> None:
        bot = FakeBot()
        chunk_calls: list[tuple[str, str]] = []
        slash_commands.register_basic_slash_commands(bot, self.make_deps(chunk_calls=chunk_calls))
        interaction = FakeInteraction(channel_id=555)

        await bot.tree.commands["usage"](interaction, 99)

        self.assertEqual(chunk_calls, [("Usage", "usage:30")])

    async def test_settings_autocomplete_uses_app_models_and_selected_model_efforts(self) -> None:
        model_catalog: slash_commands.SettingsModelCatalog = {
            "data": [
                {
                    "model": "gpt-5.6-terra",
                    "hidden": False,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "high"},
                        {"reasoningEffort": "max"},
                    ],
                },
                {
                    "model": "gpt-5.6-luna",
                    "hidden": False,
                    "supportedReasoningEfforts": [{"reasoningEffort": "ultra"}],
                },
            ]
        }
        bot = FakeBot()
        slash_commands.register_basic_slash_commands(
            bot,
            self.make_deps(model_catalog=model_catalog),
        )

        model_choices = await bot.tree.autocomplete["settings"]["model"](
            FakeInteraction(),
            "luna",
        )
        effort_choices = await bot.tree.autocomplete["settings"]["effort"](
            FakeInteraction(model="gpt-5.6-terra"),
            "",
        )

        self.assertEqual(
            [(choice.name, choice.value) for choice in model_choices],
            [("gpt-5.6-luna", "gpt-5.6-luna")],
        )
        self.assertEqual(
            [(choice.name, choice.value) for choice in effort_choices],
            [("high", "high"), ("max", "max")],
        )

    def test_settings_registers_autocomplete_in_real_discord_command_tree(self) -> None:
        bot = RealTreeBot()

        slash_commands.register_basic_slash_commands(bot, self.make_deps())

        command = bot.command_tree.get_command("settings")
        self.assertIsInstance(command, app_commands.Command)
        assert isinstance(command, app_commands.Command)
        parameters = {parameter.name: parameter for parameter in command.parameters}
        self.assertTrue(parameters["model"].autocomplete)
        self.assertTrue(parameters["effort"].autocomplete)

    async def test_settings_autocomplete_denied_user_does_not_load_catalog(self) -> None:
        catalog_loads: list[bool] = []

        def load_catalog() -> slash_commands.SettingsModelCatalog:
            catalog_loads.append(True)
            return {"data": [{"model": "private-model", "hidden": False}]}

        bot = FakeBot()
        slash_commands.register_basic_slash_commands(
            bot,
            self.make_deps(allowed=False, model_catalog_loader=load_catalog),
        )

        choices = await bot.tree.autocomplete["settings"]["model"](FakeInteraction(), "")

        self.assertEqual(choices, [])
        self.assertEqual(catalog_loads, [])
