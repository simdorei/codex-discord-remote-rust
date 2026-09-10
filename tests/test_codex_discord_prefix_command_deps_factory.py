from __future__ import annotations

import unittest
from contextlib import AbstractAsyncContextManager
from dataclasses import dataclass, field
from types import TracebackType
from typing import Protocol

import codex_discord_prefix_command_deps_factory as deps_factory
from codex_discord_session_mirror_detail import SessionMirrorDetailMode
from codex_discord_steering import SteeringPromptResult


class FakeTyping:
    async def __aenter__(self) -> None:
        return None

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        _ = (exc_type, exc, tb)


class PrefixChannelLike(Protocol):
    @property
    def id(self) -> int: ...


class PrefixAuthorLike(Protocol):
    @property
    def id(self) -> int: ...


class PrefixChannelMessageLike(Protocol):
    @property
    def channel(self) -> PrefixChannelLike: ...


class PrefixMessageLike(PrefixChannelMessageLike, Protocol):
    @property
    def author(self) -> PrefixAuthorLike: ...


class PrefixBotLike(Protocol):
    pass


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 100


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 200


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel = field(default_factory=FakeChannel)
    author: FakeAuthor = field(default_factory=FakeAuthor)


class PrefixCommandDepsFactoryTests(unittest.TestCase):
    def test_factory_builds_prefix_command_deps_with_same_callables(self) -> None:
        async def send_chunks(target: PrefixChannelLike, text: str, *, context: str = "send_chunks") -> int:
            _ = (target, text, context)
            return 1

        async def handle_plain_ask(
            message: PrefixMessageLike,
            prompt: str,
            *,
            target_thread_id: str | None = None,
        ) -> None:
            _ = (message, prompt, target_thread_id)

        async def refresh_bridge_session(bot: PrefixBotLike, *, limit: int | None = None) -> str:
            _ = (bot, limit)
            return "refresh"

        async def sync_mirror(bot: PrefixBotLike, *, limit: int | None = None) -> str:
            _ = (bot, limit)
            return "sync"

        def build_mirror(bot: PrefixBotLike, limit: int | None = None, *, channel_id: int | None = None) -> str:
            _ = (bot, limit, channel_id)
            return "mirror"

        async def prepare_output(channel: PrefixChannelLike, target_thread_id: str | None) -> bool:
            _ = (channel, target_thread_id)
            return True

        def channel_typing(channel: PrefixChannelLike, *, context: str = "typing") -> AbstractAsyncContextManager[None]:
            _ = (channel, context)
            return FakeTyping()

        def run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            _ = (prompt, target_thread_id)
            raise NotImplementedError

        def mark_steering_handoff(target_thread_id: str | None) -> None:
            _ = target_thread_id

        async def stream_steering(
            channel: PrefixChannelLike,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None = None,
            send_final_blocks: bool = True,
        ) -> bool:
            _ = (channel, steering_result, target_thread_id, send_commentary_blocks, send_final_blocks)
            return True

        def build_context_message(
            channel_id: int | None = None,
            *,
            all_threads: bool = False,
            limit: int = 10,
        ) -> str:
            _ = (channel_id, all_threads, limit)
            return "context"

        def build_context_refresh_message(channel_id: int | None = None, *, limit: int = 10) -> str:
            _ = (channel_id, limit)
            return "refresh"

        async def build_runners_message() -> str:
            return "runners"

        async def build_system_resources_message() -> str:
            return "resources"

        async def retract_queued_ask_for_request(
            *,
            channel_id: int | None,
            user_id: int | None,
            ref: str | None,
        ) -> tuple[str, dict[str, int | bool | str]]:
            _ = (channel_id, user_id, ref)
            return "retracted", {}

        def run_bridge_command(argv: list[str]) -> tuple[int, str]:
            _ = argv
            return 0, "ok"

        def get_interactive_state_for_thread(target_thread_id: str | None) -> tuple[str, str | None, str]:
            _ = target_thread_id
            return "approval", "thread-1", "ref"

        async def send_interactive_prompt(
            channel: PrefixChannelLike,
            target_thread_id: str | None,
            target_ref: str,
            state: str,
            prompt_text: str,
            choices: list[str],
        ) -> None:
            _ = (channel, target_thread_id, target_ref, state, prompt_text, choices)

        async def run_button_qa(bot: PrefixBotLike, message: PrefixChannelMessageLike) -> str:
            _ = (bot, message)
            return "qa"

        async def run_new_thread(bot: PrefixBotLike, channel_id: int | None, prompt: str) -> tuple[int, str]:
            _ = (bot, channel_id, prompt)
            return 0, "new"

        def host_reboot_allowed_user_ids_configured() -> bool:
            return True

        factory: deps_factory.PrefixCommandDepsFactory[PrefixBotLike] = deps_factory.PrefixCommandDepsFactory(
            prompt_send_chunks=send_chunks,
            mirror_send_chunks=send_chunks,
            steer_send_chunks=send_chunks,
            status_send_chunks=send_chunks,
            queue_send_chunks=send_chunks,
            archive_send_chunks=send_chunks,
            approval_send_chunks=send_chunks,
            qa_send_chunks=send_chunks,
            new_send_chunks=send_chunks,
            host_send_chunks=send_chunks,
            handle_prefix_plain_ask=handle_plain_ask,
            get_mirrored_codex_thread_id=lambda channel_id: str(channel_id),
            describe_mirrored_project_channel=lambda channel_id: str(channel_id),
            format_log_text_len=lambda text: str(len(text)),
            format_discord_command_label=lambda text: text,
            refresh_discord_bridge_session=refresh_bridge_session,
            sync_codex_mirror=sync_mirror,
            build_mirror_list=build_mirror,
            build_mirror_check=build_mirror,
            get_session_mirror_detail_mode=lambda thread_id: (
                SessionMirrorDetailMode.SEND
            ),
            set_session_mirror_detail_mode=lambda thread_id, mode: None,
            qa_commands_enabled=lambda: True,
            resolve_selected_target=lambda: ("thread-1", "ref"),
            prepare_mapped_session_mirror_output=prepare_output,
            prepare_session_mirror_delegation=prepare_output,
            channel_typing=channel_typing,
            run_steering_prompt=run_steering_prompt,
            mark_steering_handoff=mark_steering_handoff,
            stream_steering_prompt_result_to_channel=stream_steering,
            build_where_message=lambda channel_id: str(channel_id),
            build_context_message=build_context_message,
            build_context_refresh_message=build_context_refresh_message,
            clamp_context_refresh_limit=lambda value: int(value),
            build_weekly_usage_message=lambda channel_id: str(channel_id),
            build_runners_message=build_runners_message,
            build_system_resources_message=build_system_resources_message,
            retract_queued_ask_for_request=retract_queued_ask_for_request,
            run_bridge_command=run_bridge_command,
            get_interactive_state_for_thread=get_interactive_state_for_thread,
            send_interactive_prompt=send_interactive_prompt,
            interactive_state_approval="approval",
            run_discord_button_qa=run_button_qa,
            run_discord_new_thread=run_new_thread,
            host_commands_enabled=lambda: False,
            host_reboot_allowed_user_ids_configured=host_reboot_allowed_user_ids_configured,
            log_line=lambda line: None,
            monotonic=lambda: 1.0,
        )

        self.assertIs(factory.make_prefix_prompt_deps().send_chunks, send_chunks)
        self.assertIs(factory.make_prefix_mirror_deps().build_mirror_list, build_mirror)
        self.assertIs(factory.make_prefix_steer_deps().channel_typing, channel_typing)
        self.assertIs(factory.make_prefix_status_deps().build_system_resources_message, build_system_resources_message)
        self.assertIs(factory.make_prefix_queue_deps().retract_queued_ask_for_request, retract_queued_ask_for_request)
        self.assertIs(factory.make_prefix_archive_deps().run_bridge_command, run_bridge_command)
        self.assertIs(factory.make_prefix_approval_deps().send_interactive_prompt, send_interactive_prompt)
        self.assertIs(factory.make_prefix_qa_deps().run_discord_button_qa, run_button_qa)
        self.assertIs(factory.make_prefix_new_deps().run_discord_new_thread, run_new_thread)
        self.assertIs(factory.make_prefix_host_deps().send_chunks, send_chunks)
        self.assertIs(
            factory.make_prefix_host_deps().host_reboot_allowed_user_ids_configured,
            host_reboot_allowed_user_ids_configured,
        )


if __name__ == "__main__":
    _ = unittest.main()
