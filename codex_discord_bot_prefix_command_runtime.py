from __future__ import annotations

import asyncio  # noqa: ANYIO_OK
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import Generic, Protocol, TypeVar, cast, TypeAlias

import codex_app_server_transport as app_server_transport
import codex_discord_prefix_approval_commands as discord_prefix_approval_commands
import codex_discord_prefix_archive_commands as discord_prefix_archive_commands
import codex_discord_prefix_command_deps_factory as discord_prefix_command_deps_factory
import codex_discord_prefix_host_commands as discord_prefix_host_commands
import codex_discord_prefix_mirror_commands as discord_prefix_mirror_commands
import codex_discord_prefix_new_command as discord_prefix_new_command
import codex_discord_prefix_prompt_commands as discord_prefix_prompt_commands
import codex_discord_prefix_qa_command as discord_prefix_qa_command
import codex_discord_prefix_queue_commands as discord_prefix_queue_commands
import codex_discord_prefix_resume_command as discord_prefix_resume_command
import codex_discord_prefix_status_commands as discord_prefix_status_commands
import codex_discord_prefix_steer_command as discord_prefix_steer_command
import codex_discord_resume as discord_resume
import codex_discord_session_mirror_detail_runtime as discord_session_mirror_detail_runtime
ModuleValue: TypeAlias = object


BotT = TypeVar("BotT")


class PrefixSendChunks(Protocol):
    def __call__(
        self,
        target: ModuleValue,
        text: str,
        *,
        context: str = "send_chunks",
    ) -> Awaitable[int]: ...


@dataclass(frozen=True, slots=True)
class BotPrefixCommandRuntimeDeps:
    module: ModuleType
    interactive_state_approval: str
    qa_commands_enabled: Callable[[], bool]
    host_commands_enabled: Callable[[], bool]
    host_reboot_allowed_user_ids_configured: Callable[[], bool]
    monotonic: Callable[[], float]


@dataclass(frozen=True, slots=True)
class BotPrefixCommandRuntime(Generic[BotT]):
    deps: BotPrefixCommandRuntimeDeps

    def make_prefix_command_deps_factory(
        self,
    ) -> discord_prefix_command_deps_factory.PrefixCommandDepsFactory[BotT]:
        module = self.deps.module
        send_chunks = cast(PrefixSendChunks, getattr(module, "send_prefix_chunks"))
        detail_runtime = discord_session_mirror_detail_runtime.SessionMirrorDetailRuntime(
            get_db_path=lambda: cast(Path, getattr(module, "MIRROR_DB_PATH"))
        )
        return discord_prefix_command_deps_factory.PrefixCommandDepsFactory(
            prompt_send_chunks=cast(discord_prefix_prompt_commands.SendChunksFunc, send_chunks),
            mirror_send_chunks=cast(discord_prefix_mirror_commands.SendChunksFunc, send_chunks),
            steer_send_chunks=cast(discord_prefix_steer_command.SendChunksFunc, send_chunks),
            status_send_chunks=cast(discord_prefix_status_commands.SendChunksFunc, send_chunks),
            queue_send_chunks=cast(discord_prefix_queue_commands.SendChunksFunc, send_chunks),
            archive_send_chunks=cast(discord_prefix_archive_commands.SendChunksFunc, send_chunks),
            approval_send_chunks=cast(discord_prefix_approval_commands.SendChunksFunc, send_chunks),
            qa_send_chunks=cast(discord_prefix_qa_command.SendChunksFunc, send_chunks),
            new_send_chunks=cast(discord_prefix_new_command.SendChunksFunc, send_chunks),
            host_send_chunks=cast(discord_prefix_host_commands.SendChunksFunc, send_chunks),
            handle_prefix_plain_ask=cast(
                discord_prefix_prompt_commands.HandlePlainAskFunc,
                getattr(module, "handle_prefix_plain_ask"),
            ),
            get_mirrored_codex_thread_id=cast(
                Callable[[int], str | None],
                getattr(module, "get_mirrored_codex_thread_id"),
            ),
            describe_mirrored_project_channel=cast(
                Callable[[int], str],
                getattr(module, "describe_mirrored_project_channel"),
            ),
            format_log_text_len=cast(Callable[[str], str], getattr(module, "format_log_text_len_as_text")),
            format_discord_command_label=cast(
                Callable[[str], str],
                getattr(module, "format_discord_command_label"),
            ),
            refresh_discord_bridge_session=cast(
                discord_prefix_mirror_commands.RefreshBridgeFunc,
                getattr(module, "refresh_prefix_mirror_bridge_session"),
            ),
            sync_codex_mirror=cast(
                discord_prefix_mirror_commands.SyncMirrorFunc,
                getattr(module, "sync_prefix_mirror_codex"),
            ),
            build_mirror_list=cast(
                discord_prefix_mirror_commands.BuildMirrorFunc,
                getattr(module, "build_prefix_mirror_list"),
            ),
            build_mirror_check=cast(
                discord_prefix_mirror_commands.BuildMirrorFunc,
                getattr(module, "build_prefix_mirror_check"),
            ),
            get_session_mirror_detail_mode=detail_runtime.get_mode,
            set_session_mirror_detail_mode=detail_runtime.set_mode,
            qa_commands_enabled=self.deps.qa_commands_enabled,
            resolve_selected_target=cast(
                Callable[[], tuple[str | None, str]],
                getattr(module, "resolve_selected_target"),
            ),
            prepare_mapped_session_mirror_output=cast(
                discord_prefix_command_deps_factory.PrepareSessionMirrorOutputFunc,
                getattr(module, "prepare_prefix_mapped_session_mirror_output"),
            ),
            prepare_session_mirror_delegation=cast(
                discord_prefix_command_deps_factory.PrepareSessionMirrorOutputFunc,
                getattr(module, "prepare_prefix_session_mirror_delegation"),
            ),
            channel_typing=cast(
                discord_prefix_steer_command.ChannelTypingFunc,
                getattr(module, "prefix_steer_channel_typing"),
            ),
            run_steering_prompt=cast(
                discord_prefix_steer_command.RunSteeringPromptFunc,
                getattr(module, "run_steering_prompt"),
            ),
            mark_steering_handoff=cast(
                Callable[[str | None], None],
                getattr(module, "mark_persistent_busy_steering_handoff"),
            ),
            stream_steering_prompt_result_to_channel=cast(
                discord_prefix_steer_command.StreamSteeringPromptResultFunc,
                getattr(module, "stream_prefix_steering_prompt_result_to_channel"),
            ),
            build_where_message=cast(Callable[[int | None], str], getattr(module, "build_where_message")),
            build_context_message=cast(
                discord_prefix_status_commands.BuildContextMessageFunc,
                getattr(module, "build_context_message"),
            ),
            build_context_refresh_message=cast(
                discord_prefix_status_commands.BuildContextRefreshMessageFunc,
                getattr(module, "build_context_refresh_message"),
            ),
            clamp_context_refresh_limit=cast(
                Callable[[str], int],
                getattr(module, "clamp_context_refresh_limit"),
            ),
            build_weekly_usage_message=cast(Callable[[int], str], getattr(module, "build_weekly_usage_message")),
            build_runners_message=cast(Callable[[], Awaitable[str]], getattr(module, "build_runners_message")),
            build_system_resources_message=cast(
                discord_prefix_status_commands.BuildSystemResourcesMessageFunc,
                getattr(module, "build_system_resources_message"),
            ),
            retract_queued_ask_for_request=cast(
                discord_prefix_queue_commands.RetractQueuedAskFunc,
                getattr(module, "retract_queued_ask_for_request"),
            ),
            run_bridge_command=cast(Callable[[list[str]], tuple[int, str]], getattr(module, "run_bridge_command")),
            get_interactive_state_for_thread=cast(
                Callable[[str | None], tuple[str, str | None, str]],
                getattr(module, "get_interactive_state_for_thread"),
            ),
            send_interactive_prompt=cast(
                discord_prefix_approval_commands.SendInteractivePromptFunc,
                getattr(module, "send_prefix_approval_interactive_prompt"),
            ),
            interactive_state_approval=self.deps.interactive_state_approval,
            run_discord_button_qa=cast(
                discord_prefix_qa_command.RunDiscordButtonQaFunc[BotT],
                getattr(module, "run_prefix_discord_button_qa"),
            ),
            run_discord_new_thread=cast(
                discord_prefix_new_command.RunDiscordNewThreadFunc,
                getattr(module, "run_prefix_discord_new_thread"),
            ),
            host_commands_enabled=self.deps.host_commands_enabled,
            host_reboot_allowed_user_ids_configured=self.deps.host_reboot_allowed_user_ids_configured,
            log_line=cast(Callable[[str], None], getattr(module, "log_line")),
            monotonic=self.deps.monotonic,
        )

    def make_prefix_prompt_command_deps(self) -> discord_prefix_prompt_commands.PrefixPromptCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_prompt_deps()

    def make_prefix_mirror_command_deps(self) -> discord_prefix_mirror_commands.PrefixMirrorCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_mirror_deps()

    def make_prefix_steer_command_deps(self) -> discord_prefix_steer_command.PrefixSteerCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_steer_deps()

    def make_prefix_status_command_deps(self) -> discord_prefix_status_commands.PrefixStatusCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_status_deps()

    def make_prefix_queue_command_deps(self) -> discord_prefix_queue_commands.PrefixQueueCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_queue_deps()

    def make_prefix_resume_command_deps(self) -> discord_prefix_resume_command.PrefixResumeCommandDeps:
        module = self.deps.module
        return discord_prefix_resume_command.PrefixResumeCommandDeps(
            send_chunks=cast(
                discord_prefix_resume_command.SendChunksFunc,
                getattr(module, "send_prefix_chunks"),
            ),
            recover_resident_thread_for_request=self.recover_resident_thread_for_request,
            log_line=cast(Callable[[str], None], getattr(module, "log_line")),
        )

    async def recover_resident_thread_for_request(self, channel_id: int, ref: str | None) -> str:
        module = self.deps.module
        return await asyncio.to_thread(
            discord_resume.recover_resident_thread_for_request,
            app_server_transport.DEFAULT_CLIENT,
            channel_id,
            ref,
            resolve_queue_command_target=cast(
                discord_resume.ResolveQueueTargetFunc,
                getattr(module, "resolve_queue_command_target"),
            ),
            resolve_selected_target=cast(
                discord_resume.ResolveSelectedTargetFunc,
                getattr(module, "resolve_selected_target"),
            ),
        )

    def make_prefix_archive_command_deps(self) -> discord_prefix_archive_commands.PrefixArchiveCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_archive_deps()

    def make_prefix_approval_command_deps(
        self,
    ) -> discord_prefix_approval_commands.PrefixApprovalCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_approval_deps()

    def make_prefix_qa_command_deps(self) -> discord_prefix_qa_command.PrefixQaCommandDeps[BotT]:
        return self.make_prefix_command_deps_factory().make_prefix_qa_deps()

    def make_prefix_new_command_deps(self) -> discord_prefix_new_command.PrefixNewCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_new_deps()

    def make_prefix_host_command_deps(self) -> discord_prefix_host_commands.PrefixHostCommandDeps:
        return self.make_prefix_command_deps_factory().make_prefix_host_deps()
