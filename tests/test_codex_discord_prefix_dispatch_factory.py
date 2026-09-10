from __future__ import annotations

import unittest
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import cast

import codex_discord_prefix_approval_commands as prefix_approval_commands
import codex_discord_prefix_archive_commands as prefix_archive_commands
import codex_discord_prefix_dispatch as prefix_dispatch
import codex_discord_prefix_host_commands as prefix_host_commands
import codex_discord_prefix_mirror_commands as prefix_mirror_commands
import codex_discord_prefix_new_command as prefix_new_command
import codex_discord_prefix_prompt_commands as prefix_prompt_commands
import codex_discord_prefix_qa_command as prefix_qa_command
import codex_discord_prefix_queue_commands as prefix_queue_commands
import codex_discord_prefix_resume_command as prefix_resume_command
import codex_discord_prefix_status_commands as prefix_status_commands
import codex_discord_prefix_steer_command as prefix_steer_command


@dataclass(frozen=True)
class FakeChannel:
    id: int = 123


@dataclass(frozen=True)
class FakeAuthor:
    id: int = 456


@dataclass(frozen=True)
class FakeGuild:
    id: int = 789


@dataclass(frozen=True)
class FakeMessage:
    channel: FakeChannel = FakeChannel()
    author: FakeAuthor = FakeAuthor()
    guild: FakeGuild | None = FakeGuild()


@dataclass(frozen=True)
class FakeBot:
    pass


class PrefixDispatchFactoryTests(unittest.IsolatedAsyncioTestCase):
    async def test_build_prefix_dispatch_deps_wires_handlers_in_order(self) -> None:
        calls: list[tuple[str, object, object | None]] = []
        bot = FakeBot()
        markers = {
            "host": object(),
            "resume": object(),
            "steer": object(),
            "status": object(),
            "queue": object(),
            "mirror": object(),
            "approval": object(),
            "archive": object(),
            "qa": object(),
            "new": object(),
            "prompt": object(),
        }

        def make_fake(name: str) -> Callable[..., Awaitable[bool]]:
            async def fake_handler(*args: object, **kwargs: object) -> bool:
                bot_arg = args[3] if len(args) > 3 else None
                calls.append((name, kwargs["deps"], bot_arg))
                return False

            return fake_handler

        async def send_chunks(
            target: prefix_dispatch.PrefixDispatchChannel,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> int:
            return len(text) + target.id + len(context)

        def build_bridge_action(command: str, arg: str, channel_id: int) -> None:
            _ = (command, arg, channel_id)
            return None

        async def run_bridge_action(
            target: prefix_dispatch.PrefixDispatchChannel,
            argv: list[str],
            title: str,
        ) -> tuple[int, str]:
            return target.id + len(argv), title

        async def build_doctor_message(message: prefix_dispatch.PrefixDispatchMessage) -> str:
            return str(message.channel.id)

        async def run_doctor_bridge(target: prefix_dispatch.PrefixDispatchChannel) -> tuple[int, str]:
            return target.id, "doctor"

        original_steer = prefix_steer_command.handle_prefix_steer_command
        original_status = prefix_status_commands.handle_prefix_status_command
        original_queue = prefix_queue_commands.handle_prefix_queue_command
        original_resume = prefix_resume_command.handle_prefix_resume_command
        original_mirror = prefix_mirror_commands.handle_prefix_mirror_command
        original_approval = prefix_approval_commands.handle_prefix_approval_command
        original_archive = prefix_archive_commands.handle_prefix_archive_command
        original_qa = prefix_qa_command.handle_prefix_qa_command
        original_new = prefix_new_command.handle_prefix_new_command
        original_prompt = prefix_prompt_commands.handle_prefix_prompt_command
        original_host = prefix_host_commands.handle_prefix_host_command
        try:
            prefix_host_commands.handle_prefix_host_command = make_fake("host")
            prefix_steer_command.handle_prefix_steer_command = make_fake("steer")
            prefix_status_commands.handle_prefix_status_command = make_fake("status")
            prefix_queue_commands.handle_prefix_queue_command = make_fake("queue")
            prefix_resume_command.handle_prefix_resume_command = make_fake("resume")
            prefix_mirror_commands.handle_prefix_mirror_command = make_fake("mirror")
            prefix_approval_commands.handle_prefix_approval_command = make_fake("approval")
            prefix_archive_commands.handle_prefix_archive_command = make_fake("archive")
            prefix_qa_command.handle_prefix_qa_command = make_fake("qa")
            prefix_new_command.handle_prefix_new_command = make_fake("new")
            prefix_prompt_commands.handle_prefix_prompt_command = make_fake("prompt")

            factory = prefix_dispatch.PrefixDispatchFactoryDeps(
                bot=bot,
                send_chunks=send_chunks,
                build_help=lambda: "help",
                build_bridge_action=build_bridge_action,
                run_bridge_action=run_bridge_action,
                build_doctor_message=build_doctor_message,
                run_doctor_bridge=run_doctor_bridge,
                format_command_label=str,
                make_prefix_steer_deps=lambda: cast(prefix_steer_command.PrefixSteerCommandDeps, markers["steer"]),
                make_prefix_status_deps=lambda: cast(prefix_status_commands.PrefixStatusCommandDeps, markers["status"]),
                make_prefix_queue_deps=lambda: cast(prefix_queue_commands.PrefixQueueCommandDeps, markers["queue"]),
                make_prefix_resume_deps=lambda: cast(
                    prefix_resume_command.PrefixResumeCommandDeps,
                    markers["resume"],
                ),
                make_prefix_mirror_deps=lambda: cast(prefix_mirror_commands.PrefixMirrorCommandDeps, markers["mirror"]),
                make_prefix_approval_deps=lambda: cast(
                    prefix_approval_commands.PrefixApprovalCommandDeps,
                    markers["approval"],
                ),
                make_prefix_archive_deps=lambda: cast(
                    prefix_archive_commands.PrefixArchiveCommandDeps,
                    markers["archive"],
                ),
                make_prefix_qa_deps=lambda: cast(prefix_qa_command.PrefixQaCommandDeps[FakeBot], markers["qa"]),
                make_prefix_new_deps=lambda: cast(prefix_new_command.PrefixNewCommandDeps, markers["new"]),
                make_prefix_prompt_deps=lambda: cast(
                    prefix_prompt_commands.PrefixPromptCommandDeps,
                    markers["prompt"],
                ),
                make_prefix_host_deps=lambda: cast(prefix_host_commands.PrefixHostCommandDeps, markers["host"]),
            )
            deps = prefix_dispatch.build_prefix_dispatch_deps(factory)

            for handler in deps.handlers:
                _ = await handler("cmd", "arg", FakeMessage())
        finally:
            prefix_steer_command.handle_prefix_steer_command = original_steer
            prefix_status_commands.handle_prefix_status_command = original_status
            prefix_queue_commands.handle_prefix_queue_command = original_queue
            prefix_resume_command.handle_prefix_resume_command = original_resume
            prefix_mirror_commands.handle_prefix_mirror_command = original_mirror
            prefix_approval_commands.handle_prefix_approval_command = original_approval
            prefix_archive_commands.handle_prefix_archive_command = original_archive
            prefix_qa_command.handle_prefix_qa_command = original_qa
            prefix_new_command.handle_prefix_new_command = original_new
            prefix_prompt_commands.handle_prefix_prompt_command = original_prompt
            prefix_host_commands.handle_prefix_host_command = original_host

        self.assertEqual(
            [name for name, _deps, _bot in calls],
            ["host", "resume", "steer", "status", "queue", "mirror", "approval", "archive", "qa", "new", "prompt"],
        )
        self.assertEqual([deps for _name, deps, _bot in calls], list(markers.values()))
        self.assertEqual(
            [bot_arg for _name, _deps, bot_arg in calls],
            [None, None, None, None, None, bot, None, None, bot, bot, None],
        )


if __name__ == "__main__":
    _ = unittest.main()
