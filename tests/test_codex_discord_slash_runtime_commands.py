from __future__ import annotations

import unittest
from collections.abc import Callable

import codex_discord_slash_commands as slash_commands
import codex_discord_slash_runtime_commands as runtime_commands

SlashFunc = slash_commands.SlashCallback


class FakeTree:
    def __init__(self) -> None:
        self.commands: dict[str, SlashFunc] = {}

    def command(self, *, name: str, description: str) -> Callable[[SlashFunc], SlashFunc]:
        def decorate(func: SlashFunc) -> SlashFunc:
            _ = description
            self.commands[name] = func
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


class FakeUser:
    def __init__(self, user_id: int = 777) -> None:
        self.id: int = user_id


class FakeChannel:
    def __init__(self, channel_id: int = 888) -> None:
        self.id: int = channel_id


class FakeInteraction:
    def __init__(
        self,
        *,
        channel_id: int = 222,
        channel: FakeChannel | None = None,
        channel_available: bool = True,
        user: FakeUser | None = None,
    ) -> None:
        self.channel_id: int = channel_id
        self.channel: FakeChannel | None = (FakeChannel() if channel is None else channel) if channel_available else None
        self.response: FakeResponse = FakeResponse()
        self.user: FakeUser = FakeUser() if user is None else user


class RuntimeSlashCommandRegistrationTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        allowed: bool = True,
        qa_enabled: bool = True,
        denied_calls: list[runtime_commands.RuntimeSlashInteraction] | None = None,
        chunk_calls: list[tuple[str, str]] | None = None,
        bridge_calls: list[tuple[list[str], str]] | None = None,
        doctor_calls: list[tuple[int | None, bool]] | None = None,
        retract_calls: list[tuple[int | None, int | None, str | None]] | None = None,
        mirror_error: RuntimeError | None = None,
        bridge_sync_error: RuntimeError | None = None,
        response_calls: list[tuple[str, bool, str]] | None = None,
        qa_calls: list[tuple[int, int]] | None = None,
        log_calls: list[str] | None = None,
    ) -> runtime_commands.RuntimeSlashCommandDeps:
        async def send_not_allowed(
            interaction: runtime_commands.RuntimeSlashInteraction,
        ) -> None:
            if denied_calls is not None:
                denied_calls.append(interaction)

        async def send_chunks(
            interaction: runtime_commands.RuntimeSlashInteraction,
            text: str,
            *,
            title: str,
        ) -> None:
            _ = interaction
            if chunk_calls is not None:
                chunk_calls.append((title, text))

        async def run_bridge(
            interaction: runtime_commands.RuntimeSlashInteraction,
            argv: list[str],
            title: str,
        ) -> tuple[int, str]:
            _ = interaction
            if bridge_calls is not None:
                bridge_calls.append((argv, title))
            return 0, "bridge ok"

        async def build_doctor(
            bot: slash_commands.SlashCommandBot,
            channel_id: int | None,
            channel: runtime_commands.RuntimeSlashChannel | None,
        ) -> str:
            _ = bot
            if doctor_calls is not None:
                doctor_calls.append((channel_id, channel is None))
            return f"doctor:{channel_id}:{channel is None}"

        async def build_runners() -> str:
            return "runners text"

        async def retract_queued_ask(
            *,
            channel_id: int | None,
            user_id: int | None,
            ref: str | None,
        ) -> tuple[str, runtime_commands.QueueRetractResult]:
            if retract_calls is not None:
                retract_calls.append((channel_id, user_id, ref))
            return "retract response", {"removed": 1}

        async def run_mirror_check() -> str:
            if mirror_error is not None:
                raise mirror_error
            return "mirror ok"

        async def refresh_bridge_session(
            bot: slash_commands.SlashCommandBot,
            *,
            limit: int | None = None,
        ) -> str:
            _ = bot
            if bridge_sync_error is not None:
                raise bridge_sync_error
            return f"bridge sync:{limit}"

        async def send_response(
            interaction: runtime_commands.RuntimeSlashInteraction,
            content: str,
            *,
            ephemeral: bool = False,
            context: str = "interaction_response",
        ) -> None:
            _ = interaction
            if response_calls is not None:
                response_calls.append((content, ephemeral, context))

        async def run_button_qa(
            bot: slash_commands.SlashCommandBot,
            message: runtime_commands.RuntimeSlashSourceMessage,
        ) -> str:
            _ = bot
            if qa_calls is not None:
                qa_calls.append((message.author.id, message.channel.id))
            return "qa output"

        def log_line(line: str) -> None:
            if log_calls is not None:
                log_calls.append(line)

        return runtime_commands.RuntimeSlashCommandDeps(
            check_allowed=lambda interaction: allowed,
            send_not_allowed=send_not_allowed,
            send_chunks=send_chunks,
            run_bridge=run_bridge,
            build_doctor=build_doctor,
            build_runners=build_runners,
            retract_queued_ask=retract_queued_ask,
            run_mirror_check=run_mirror_check,
            refresh_bridge_session=refresh_bridge_session,
            qa_commands_enabled=lambda: qa_enabled,
            send_response=send_response,
            run_button_qa=run_button_qa,
            log_line=log_line,
        )

    def test_registers_runtime_slash_command_names_when_qa_enabled(self) -> None:
        bot = FakeBot()
        runtime_commands.register_runtime_slash_commands(bot, self.make_deps(qa_enabled=True))

        self.assertEqual(
            set(bot.tree.commands),
            {"bridge_sync", "doctor", "mirror_check", "qa_buttons", "retract", "runners"},
        )

    def test_omits_qa_buttons_when_qa_commands_disabled(self) -> None:
        bot = FakeBot()
        runtime_commands.register_runtime_slash_commands(bot, self.make_deps(qa_enabled=False))

        self.assertEqual(set(bot.tree.commands), {"bridge_sync", "doctor", "mirror_check", "retract", "runners"})

    async def test_denied_interaction_sends_not_allowed_without_defer(self) -> None:
        bot = FakeBot()
        denied_calls: list[runtime_commands.RuntimeSlashInteraction] = []
        chunk_calls: list[tuple[str, str]] = []
        runtime_commands.register_runtime_slash_commands(
            bot, self.make_deps(allowed=False, denied_calls=denied_calls, chunk_calls=chunk_calls)
        )
        interaction = FakeInteraction()

        await bot.tree.commands["runners"](interaction)

        self.assertEqual(denied_calls, [interaction])
        self.assertFalse(interaction.response.deferred)
        self.assertEqual(chunk_calls, [])

    async def test_doctor_sends_diagnostic_chunks_and_runs_bridge(self) -> None:
        bot = FakeBot()
        chunk_calls: list[tuple[str, str]] = []
        bridge_calls: list[tuple[list[str], str]] = []
        doctor_calls: list[tuple[int | None, bool]] = []
        runtime_commands.register_runtime_slash_commands(
            bot, self.make_deps(chunk_calls=chunk_calls, bridge_calls=bridge_calls, doctor_calls=doctor_calls)
        )
        interaction = FakeInteraction(channel_id=333, channel=FakeChannel(444))

        await bot.tree.commands["doctor"](interaction)

        self.assertEqual(doctor_calls, [(333, False)])
        self.assertEqual(chunk_calls, [("Discord doctor", "doctor:333:False")])
        self.assertEqual(bridge_calls, [(["doctor"], "Doctor")])

    async def test_retract_normalizes_empty_ref_to_none(self) -> None:
        bot = FakeBot()
        retract_calls: list[tuple[int | None, int | None, str | None]] = []
        chunk_calls: list[tuple[str, str]] = []
        runtime_commands.register_runtime_slash_commands(
            bot, self.make_deps(retract_calls=retract_calls, chunk_calls=chunk_calls)
        )
        interaction = FakeInteraction(channel_id=555, user=FakeUser(999))

        await bot.tree.commands["retract"](interaction, "")

        self.assertEqual(retract_calls, [(555, 999, None)])
        self.assertEqual(chunk_calls, [("Retract", "retract response")])

    async def test_mirror_check_exception_is_logged_and_sent(self) -> None:
        bot = FakeBot()
        chunk_calls: list[tuple[str, str]] = []
        log_calls: list[str] = []
        runtime_commands.register_runtime_slash_commands(
            bot, self.make_deps(chunk_calls=chunk_calls, mirror_error=RuntimeError("mirror boom"), log_calls=log_calls)
        )
        interaction = FakeInteraction()

        await bot.tree.commands["mirror_check"](interaction)

        self.assertEqual(chunk_calls, [("Mirror check", "Mirror check failed\n\nERROR: mirror boom")])
        self.assertIn("slash_mirror_check_failed", log_calls[0])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])

    async def test_bridge_sync_exception_is_logged_and_sent(self) -> None:
        bot = FakeBot()
        chunk_calls: list[tuple[str, str]] = []
        log_calls: list[str] = []
        runtime_commands.register_runtime_slash_commands(
            bot,
            self.make_deps(chunk_calls=chunk_calls, bridge_sync_error=RuntimeError("sync boom"), log_calls=log_calls),
        )
        interaction = FakeInteraction()

        await bot.tree.commands["bridge_sync"](interaction, 12)

        self.assertEqual(chunk_calls, [("Bridge sync", "Discord bridge sync failed\n\nERROR: sync boom")])
        self.assertIn("slash_bridge_sync_failed", log_calls[0])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])

    async def test_qa_buttons_channel_unavailable_uses_tracked_response(self) -> None:
        bot = FakeBot()
        response_calls: list[tuple[str, bool, str]] = []
        qa_calls: list[tuple[int, int]] = []
        runtime_commands.register_runtime_slash_commands(
            bot, self.make_deps(response_calls=response_calls, qa_calls=qa_calls)
        )
        interaction = FakeInteraction(channel_available=False)

        await bot.tree.commands["qa_buttons"](interaction)

        self.assertEqual(response_calls, [("Discord channel is unavailable.", True, "qa_buttons_channel_unavailable")])
        self.assertEqual(qa_calls, [])
        self.assertFalse(interaction.response.deferred)

    async def test_qa_buttons_runs_button_qa_for_available_channel(self) -> None:
        bot = FakeBot()
        qa_calls: list[tuple[int, int]] = []
        chunk_calls: list[tuple[str, str]] = []
        runtime_commands.register_runtime_slash_commands(bot, self.make_deps(qa_calls=qa_calls, chunk_calls=chunk_calls))
        interaction = FakeInteraction(channel=FakeChannel(123), user=FakeUser(456))

        await bot.tree.commands["qa_buttons"](interaction)

        self.assertEqual(qa_calls, [(456, 123)])
        self.assertEqual(chunk_calls, [("Discord button QA", "qa output")])
        self.assertEqual(interaction.response.defer_kwargs, [{"thinking": True}])
