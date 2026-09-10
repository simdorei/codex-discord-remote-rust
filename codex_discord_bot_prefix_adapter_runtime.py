from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from types import ModuleType
from typing import cast, TypeAlias

import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_dispatch as discord_prefix_dispatch
import codex_discord_prefix_dispatch_runtime as discord_prefix_dispatch_runtime
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_resume_command as discord_prefix_resume_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command
import codex_system_resources
from codex_discord_steering import SteeringPromptResult
ModuleValue: TypeAlias = object


class PrefixApprovalTargetMissingError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class BotPrefixAdapterRuntime:
    module: ModuleType

    async def send_prefix_chunks(
        self,
        target: ModuleValue,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> int:
        require_messageable = cast(
            Callable[[object], discord_prefix_dispatch.PrefixDispatchChannel],
            getattr(self.module, "require_discord_messageable"),
        )
        send_chunks = cast(Callable[..., Awaitable[int]], getattr(self.module, "send_chunks"))
        return await send_chunks(require_messageable(target), text, context=context)

    async def handle_prefix_plain_ask(
        self,
        message: discord_prefix_prompt_commands.MessageLike,
        prompt: str,
        *,
        target_thread_id: str | None = None,
    ) -> None:
        handle_plain_ask = cast(Callable[..., Awaitable[None]], getattr(self.module, "handle_plain_ask"))
        await handle_plain_ask(cast(object, message), prompt, target_thread_id=target_thread_id)

    async def stream_prefix_steering_prompt_result_to_channel(
        self,
        channel: ModuleValue,
        steering_result: SteeringPromptResult,
        target_thread_id: str | None,
        *,
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> bool:
        require_messageable = cast(
            Callable[[object], discord_prefix_dispatch.PrefixDispatchChannel],
            getattr(self.module, "require_discord_messageable"),
        )
        stream_result = cast(Callable[..., Awaitable[bool]], getattr(self.module, "stream_steering_prompt_result_to_channel"))
        return await stream_result(
            require_messageable(channel),
            steering_result,
            target_thread_id,
            send_commentary_blocks=send_commentary_blocks,
            send_final_blocks=send_final_blocks,
        )

    async def refresh_prefix_mirror_bridge_session(
        self,
        bot: discord_prefix_mirror_commands.MirrorCommandBot,
        *,
        limit: int | None = None,
    ) -> str:
        refresh_session = cast(Callable[..., Awaitable[str]], getattr(self.module, "refresh_discord_bridge_session"))
        return await refresh_session(bot, limit=limit)

    async def sync_prefix_mirror_codex(
        self,
        bot: discord_prefix_mirror_commands.MirrorCommandBot,
        *,
        limit: int | None = None,
    ) -> str:
        sync_mirror = cast(Callable[..., Awaitable[str]], getattr(self.module, "sync_codex_mirror"))
        return await sync_mirror(bot, limit=limit)

    async def send_prefix_approval_interactive_prompt(
        self,
        channel: discord_prefix_approval_commands.ChannelLike,
        target_thread_id: str | None,
        target_ref: str,
        state: str,
        prompt_text: str,
        choices: list[str],
    ) -> None:
        if target_thread_id is None:
            raise PrefixApprovalTargetMissingError("prefix approval interactive prompt requires target_thread_id")
        require_messageable = cast(
            Callable[[object], discord_prefix_dispatch.PrefixDispatchChannel],
            getattr(self.module, "require_discord_messageable"),
        )
        send_interactive_prompt = cast(Callable[..., Awaitable[None]], getattr(self.module, "send_interactive_prompt"))
        await send_interactive_prompt(
            require_messageable(channel),
            target_thread_id,
            target_ref,
            state,
            prompt_text,
            [(choice, choice) for choice in choices],
        )

    async def build_system_resources_message(self) -> str:
        return await asyncio.to_thread(codex_system_resources.build_system_resources_message)

    def make_prefix_dispatch_deps(
        self,
        bot: discord_prefix_dispatch.PrefixDispatchBot,
    ) -> discord_prefix_dispatch.PrefixDispatchDeps:
        return discord_prefix_dispatch_runtime.make_prefix_dispatch_deps(
            bot,
            deps=discord_prefix_dispatch_runtime.PrefixDispatchRuntimeDeps(
                send_prefix_chunks=self.send_prefix_chunks,
                build_help=cast(Callable[[], str], getattr(self.module, "build_help")),
                resolve_thread_target_args=cast(
                    Callable[[int | None, str | None], list[str]],
                    getattr(self.module, "resolve_discord_thread_target_args"),
                ),
                resolve_archive_target_args=cast(
                    Callable[[int | None, str | None], list[str]],
                    getattr(self.module, "resolve_discord_archive_target_args"),
                ),
                run_bridge_and_send=cast(
                    discord_prefix_dispatch_runtime.BridgeRunner[
                        discord_prefix_dispatch.PrefixDispatchChannel,
                        discord_prefix_dispatch.PrefixDispatchBot,
                    ],
                    getattr(self.module, "run_bridge_and_send"),
                ),
                require_messageable=cast(
                    Callable[
                        [discord_prefix_dispatch.PrefixDispatchChannel],
                        discord_prefix_dispatch.PrefixDispatchChannel,
                    ],
                    getattr(self.module, "require_discord_messageable"),
                ),
                build_doctor_message_with_history=cast(
                    discord_prefix_dispatch_runtime.DoctorMessageBuilder[
                        discord_prefix_dispatch.PrefixDispatchBot,
                        discord_prefix_dispatch.PrefixDispatchChannel,
                    ],
                    getattr(self.module, "build_discord_doctor_message_with_history"),
                ),
                require_history_channel=cast(
                    Callable[
                        [discord_prefix_dispatch.PrefixDispatchChannel],
                        discord_prefix_dispatch.PrefixDispatchChannel,
                    ],
                    getattr(self.module, "require_discord_history_channel"),
                ),
                format_command_label=cast(Callable[[str], str], getattr(self.module, "format_discord_command_label")),
                make_prefix_steer_deps=cast(
                    Callable[[], discord_prefix_steer_command.PrefixSteerCommandDeps],
                    getattr(self.module, "_make_prefix_steer_command_deps"),
                ),
                make_prefix_status_deps=cast(
                    Callable[[], discord_prefix_status_commands.PrefixStatusCommandDeps],
                    getattr(self.module, "_make_prefix_status_command_deps"),
                ),
                make_prefix_queue_deps=cast(
                    Callable[[], discord_prefix_queue_commands.PrefixQueueCommandDeps],
                    getattr(self.module, "_make_prefix_queue_command_deps"),
                ),
                make_prefix_resume_deps=cast(
                    Callable[[], discord_prefix_resume_command.PrefixResumeCommandDeps],
                    getattr(self.module, "_make_prefix_resume_command_deps"),
                ),
                make_prefix_mirror_deps=cast(
                    Callable[[], discord_prefix_mirror_commands.PrefixMirrorCommandDeps],
                    getattr(self.module, "_make_prefix_mirror_command_deps"),
                ),
                make_prefix_approval_deps=cast(
                    Callable[[], discord_prefix_approval_commands.PrefixApprovalCommandDeps],
                    getattr(self.module, "_make_prefix_approval_command_deps"),
                ),
                make_prefix_archive_deps=cast(
                    Callable[[], discord_prefix_archive_commands.PrefixArchiveCommandDeps],
                    getattr(self.module, "_make_prefix_archive_command_deps"),
                ),
                make_prefix_qa_deps=cast(
                    Callable[[], discord_prefix_qa_command.PrefixQaCommandDeps[discord_prefix_dispatch.PrefixDispatchBot]],
                    getattr(self.module, "_make_prefix_qa_command_deps"),
                ),
                make_prefix_new_deps=cast(
                    Callable[[], discord_prefix_new_command.PrefixNewCommandDeps],
                    getattr(self.module, "_make_prefix_new_command_deps"),
                ),
                make_prefix_prompt_deps=cast(
                    Callable[[], discord_prefix_prompt_commands.PrefixPromptCommandDeps],
                    getattr(self.module, "_make_prefix_prompt_command_deps"),
                ),
                make_prefix_host_deps=cast(
                    Callable[[], discord_prefix_host_commands.PrefixHostCommandDeps],
                    getattr(self.module, "_make_prefix_host_command_deps"),
                ),
            ),
        )
