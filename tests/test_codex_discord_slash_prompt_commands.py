from __future__ import annotations

import unittest
from collections.abc import Callable
from dataclasses import dataclass, fields

import codex_discord_slash_commands as slash_commands
import codex_discord_slash_prompt_commands as slash_prompt_commands

SlashFunc = slash_commands.SlashCallback


class FakeTree:
    def __init__(self) -> None:
        self.commands: dict[str, SlashFunc] = {}
        self.descriptions: dict[str, str] = {}

    def command(self, *, name: str, description: str) -> Callable[[SlashFunc], SlashFunc]:
        def decorate(func: SlashFunc) -> SlashFunc:
            self.commands[name] = func
            self.descriptions[name] = description
            return func

        return decorate


class FakeBot:
    def __init__(self) -> None:
        self._tree: FakeTree = FakeTree()

    @property
    def tree(self) -> FakeTree:
        return self._tree


class FakeResponse:
    def __init__(self) -> None:
        self.deferred: bool = False
        self.defer_kwargs: list[dict[str, bool]] = []

    async def defer(self, thinking: bool = False, **kwargs: bool) -> None:
        self.deferred = True
        self.defer_kwargs.append({"thinking": thinking, **kwargs})


class FakeInteraction:
    def __init__(self, channel_id: int = 222) -> None:
        self.channel_id: int = channel_id
        self.response: FakeResponse = FakeResponse()
        self.channel: FakeChannel | None = FakeChannel()
        self.user: FakeUser = FakeUser()


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222

    async def send(self, content: str) -> None:
        _ = content


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int = 333


@dataclass(frozen=True, slots=True)
class FakeSourceMessage:
    channel: slash_prompt_commands.PromptChannel
    author: slash_prompt_commands.PromptUser


class PromptSlashCommandRegistrationTests(unittest.IsolatedAsyncioTestCase):
    def make_handler(
        self,
        name: str,
        events: list[tuple[str, str]],
    ) -> slash_prompt_commands.PromptSlashHandler:
        async def handle(
            interaction: slash_prompt_commands.PromptSlashInteraction,
            prompt: str,
        ) -> None:
            _ = interaction
            events.append((name, prompt))

        return handle

    def make_deps(
        self,
        *,
        allowed: bool = True,
        events: list[tuple[str, str]] | None = None,
        denied_calls: list[slash_prompt_commands.PromptSlashInteraction] | None = None,
    ) -> slash_prompt_commands.PromptSlashCommandDeps:
        if events is None:
            events = []

        async def send_not_allowed(
            interaction: slash_prompt_commands.PromptSlashInteraction,
        ) -> None:
            if denied_calls is not None:
                denied_calls.append(interaction)

        return slash_prompt_commands.PromptSlashCommandDeps(
            check_allowed=lambda interaction: allowed,
            send_not_allowed=send_not_allowed,
            handle_new=self.make_handler("new", events),
            handle_ask=self.make_handler("ask", events),
            handle_interview=self.make_handler("interview", events),
        )

    def test_registers_prompt_slash_command_names(self) -> None:
        bot = FakeBot()
        slash_prompt_commands.register_prompt_slash_commands(bot, self.make_deps())

        self.assertEqual(
            set(bot.tree.commands),
            {
                "ask",
                "ask_ipc",
                "interview",
                "new",
            },
        )
        dep_fields = {field.name for field in fields(slash_prompt_commands.PromptSlashCommandDeps)}
        self.assertNotIn("handle_github_triage", dep_fields)
        self.assertNotIn("handle_maintainer_orchestrator", dep_fields)

    async def test_allowed_new_defers_and_calls_new_handler(self) -> None:
        bot = FakeBot()
        events: list[tuple[str, str]] = []
        slash_prompt_commands.register_prompt_slash_commands(bot, self.make_deps(events=events))
        interaction = FakeInteraction()

        await bot.tree.commands["new"](interaction, "start here")

        self.assertEqual(events, [("new", "start here")])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])

    async def test_denied_interaction_sends_not_allowed_without_defer(self) -> None:
        bot = FakeBot()
        events: list[tuple[str, str]] = []
        denied_calls: list[slash_prompt_commands.PromptSlashInteraction] = []
        slash_prompt_commands.register_prompt_slash_commands(
            bot,
            self.make_deps(allowed=False, events=events, denied_calls=denied_calls),
        )
        interaction = FakeInteraction()

        await bot.tree.commands["ask"](interaction, "blocked prompt")

        self.assertEqual(denied_calls, [interaction])
        self.assertEqual(events, [])
        self.assertFalse(interaction.response.deferred)

    async def test_ask_ipc_aliases_ask_handler(self) -> None:
        bot = FakeBot()
        events: list[tuple[str, str]] = []
        slash_prompt_commands.register_prompt_slash_commands(bot, self.make_deps(events=events))
        interaction = FakeInteraction()

        await bot.tree.commands["ask_ipc"](interaction, "$custom hello")

        self.assertEqual(events, [("ask", "$custom hello")])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])


class SkillSlashPromptHandlerTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        mirrored_thread_id: str | None = "thread-1",
        project_message: str = "",
    ) -> tuple[
        slash_prompt_commands.SkillSlashPromptDeps[FakeSourceMessage],
        list[tuple[str, str]],
        list[tuple[FakeSourceMessage, str, str | None]],
        list[str],
    ]:
        sent: list[tuple[str, str]] = []
        handled: list[tuple[FakeSourceMessage, str, str | None]] = []
        logs: list[str] = []

        async def send_chunks(
            interaction: slash_prompt_commands.SkillSlashInteraction,
            text: str,
            *,
            title: str,
        ) -> None:
            _ = interaction
            sent.append((title, text))

        async def send_direct_followup(
            interaction: slash_prompt_commands.SkillSlashInteraction,
            text: str,
            *,
            ephemeral: bool,
            log_prefix: str,
            context: str,
        ) -> None:
            _ = interaction, ephemeral, log_prefix
            sent.append((context, text))

        async def handle_plain_ask(
            source_message: FakeSourceMessage,
            prompt: str,
            *,
            target_thread_id: str | None,
        ) -> None:
            handled.append((source_message, prompt, target_thread_id))

        deps = slash_prompt_commands.SkillSlashPromptDeps(
            send_interaction_chunks=send_chunks,
            send_direct_followup=send_direct_followup,
            handle_plain_ask=handle_plain_ask,
            get_mirrored_codex_thread_id=lambda channel_id: mirrored_thread_id,
            describe_mirrored_project_channel=lambda channel_id: project_message,
            get_interaction_command_name=lambda interaction: "interview",
            format_log_text_len=lambda text: str(len(text)),
            make_source_message=lambda channel, user: FakeSourceMessage(channel=channel, author=user),
            log_line=logs.append,
        )
        return deps, sent, handled, logs

    async def test_skill_prompt_dispatches_wrapped_prompt_to_mapped_thread(self) -> None:
        deps, sent, handled, logs = self.make_deps()
        spec = slash_prompt_commands.SkillSlashPromptSpec(
            title="Interview",
            log_name="slash_interview",
            ack_message="Interview handling posted in this channel.",
            ack_context="interview_posted",
            build_prompt=lambda prompt: f"wrapped:{prompt}",
        )

        await slash_prompt_commands.handle_skill_slash_prompt(
            FakeInteraction(),
            "build dashboard",
            spec=spec,
            deps=deps,
        )

        self.assertEqual(sent, [("interview_posted", "Interview handling posted in this channel.")])
        self.assertEqual(len(handled), 1)
        source_message, prompt, target_thread_id = handled[0]
        self.assertEqual(prompt, "wrapped:build dashboard")
        self.assertEqual(target_thread_id, "thread-1")
        self.assertEqual(source_message.channel.id, 222)
        self.assertIn("slash_interview_dispatch command=interview channel=222", "\n".join(logs))
        self.assertIn("slash_interview_ack_sent command=interview channel=222", "\n".join(logs))

    async def test_skill_prompt_blocks_project_parent_without_plain_ask(self) -> None:
        deps, sent, handled, logs = self.make_deps(
            mirrored_thread_id=None,
            project_message="Use a mirrored child thread.",
        )
        spec = slash_prompt_commands.SkillSlashPromptSpec(
            title="Interview",
            log_name="slash_interview",
            ack_message="Interview handling posted in this channel.",
            ack_context="interview_posted",
            build_prompt=lambda prompt: f"wrapped:{prompt}",
        )

        await slash_prompt_commands.handle_skill_slash_prompt(
            FakeInteraction(),
            "build dashboard",
            spec=spec,
            deps=deps,
        )

        self.assertEqual(sent, [("Interview", "Use a mirrored child thread.")])
        self.assertEqual(handled, [])
        self.assertIn("slash_interview_blocked command=interview channel=222", "\n".join(logs))
