from __future__ import annotations

from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Generic, TypeVar

import codex_discord_prefix_approval_commands as approval_commands
import codex_discord_prefix_archive_commands as archive_commands
import codex_discord_prefix_host_commands as host_commands
import codex_discord_prefix_mirror_commands as mirror_commands
import codex_discord_prefix_new_command as new_command
import codex_discord_prefix_prompt_commands as prompt_commands
import codex_discord_prefix_qa_command as qa_command
import codex_discord_prefix_queue_commands as queue_commands
import codex_discord_prefix_status_commands as status_commands
import codex_discord_prefix_steer_command as steer_command
from codex_discord_session_mirror_detail import SessionMirrorDetailMode


BotT = TypeVar("BotT")
PrepareSessionMirrorOutputFunc = Callable[[steer_command.ChannelLike, str | None], Awaitable[bool]]


@dataclass(frozen=True, slots=True)
class PrefixCommandDepsFactory(Generic[BotT]):
    prompt_send_chunks: prompt_commands.SendChunksFunc
    mirror_send_chunks: mirror_commands.SendChunksFunc
    steer_send_chunks: steer_command.SendChunksFunc
    status_send_chunks: status_commands.SendChunksFunc
    queue_send_chunks: queue_commands.SendChunksFunc
    archive_send_chunks: archive_commands.SendChunksFunc
    approval_send_chunks: approval_commands.SendChunksFunc
    qa_send_chunks: qa_command.SendChunksFunc
    new_send_chunks: new_command.SendChunksFunc
    host_send_chunks: host_commands.SendChunksFunc
    handle_prefix_plain_ask: prompt_commands.HandlePlainAskFunc
    get_mirrored_codex_thread_id: Callable[[int], str | None]
    describe_mirrored_project_channel: Callable[[int], str]
    format_log_text_len: Callable[[str], str]
    format_discord_command_label: Callable[[str], str]
    refresh_discord_bridge_session: mirror_commands.RefreshBridgeFunc
    sync_codex_mirror: mirror_commands.SyncMirrorFunc
    build_mirror_list: mirror_commands.BuildMirrorFunc
    build_mirror_check: mirror_commands.BuildMirrorFunc
    get_session_mirror_detail_mode: Callable[[str], SessionMirrorDetailMode]
    set_session_mirror_detail_mode: Callable[
        [str, SessionMirrorDetailMode],
        None,
    ]
    qa_commands_enabled: Callable[[], bool]
    resolve_selected_target: Callable[[], tuple[str | None, str]]
    prepare_mapped_session_mirror_output: PrepareSessionMirrorOutputFunc
    prepare_session_mirror_delegation: PrepareSessionMirrorOutputFunc
    channel_typing: steer_command.ChannelTypingFunc
    run_steering_prompt: steer_command.RunSteeringPromptFunc
    mark_steering_handoff: Callable[[str | None], None]
    stream_steering_prompt_result_to_channel: steer_command.StreamSteeringPromptResultFunc
    build_where_message: Callable[[int | None], str]
    build_context_message: status_commands.BuildContextMessageFunc
    build_context_refresh_message: status_commands.BuildContextRefreshMessageFunc
    clamp_context_refresh_limit: Callable[[str], int]
    build_weekly_usage_message: Callable[[int], str]
    build_runners_message: Callable[[], Awaitable[str]]
    build_system_resources_message: status_commands.BuildSystemResourcesMessageFunc
    retract_queued_ask_for_request: queue_commands.RetractQueuedAskFunc
    run_bridge_command: Callable[[list[str]], tuple[int, str]]
    get_interactive_state_for_thread: Callable[[str | None], tuple[str, str | None, str]]
    send_interactive_prompt: approval_commands.SendInteractivePromptFunc
    interactive_state_approval: str
    run_discord_button_qa: qa_command.RunDiscordButtonQaFunc[BotT]
    run_discord_new_thread: new_command.RunDiscordNewThreadFunc
    host_commands_enabled: Callable[[], bool]
    host_reboot_allowed_user_ids_configured: Callable[[], bool]
    log_line: Callable[[str], None]
    monotonic: Callable[[], float]

    def make_prefix_prompt_deps(self) -> prompt_commands.PrefixPromptCommandDeps:
        return prompt_commands.PrefixPromptCommandDeps(
            send_chunks=self.prompt_send_chunks,
            handle_plain_ask=self.handle_prefix_plain_ask,
            get_mirrored_codex_thread_id=self.get_mirrored_codex_thread_id,
            describe_mirrored_project_channel=self.describe_mirrored_project_channel,
            log_line=self.log_line,
            format_log_text_len=self.format_log_text_len,
            format_discord_command_label=self.format_discord_command_label,
        )

    def make_prefix_mirror_deps(self) -> mirror_commands.PrefixMirrorCommandDeps:
        return mirror_commands.PrefixMirrorCommandDeps(
            send_chunks=self.mirror_send_chunks,
            refresh_discord_bridge_session=self.refresh_discord_bridge_session,
            sync_codex_mirror=self.sync_codex_mirror,
            build_mirror_list=self.build_mirror_list,
            build_mirror_check=self.build_mirror_check,
            get_mirrored_codex_thread_id=self.get_mirrored_codex_thread_id,
            describe_mirrored_project_channel=self.describe_mirrored_project_channel,
            get_session_mirror_detail_mode=self.get_session_mirror_detail_mode,
            set_session_mirror_detail_mode=self.set_session_mirror_detail_mode,
            log_line=self.log_line,
        )

    def make_prefix_steer_deps(self) -> steer_command.PrefixSteerCommandDeps:
        return steer_command.PrefixSteerCommandDeps(
            send_chunks=self.steer_send_chunks,
            qa_commands_enabled=self.qa_commands_enabled,
            get_mirrored_codex_thread_id=self.get_mirrored_codex_thread_id,
            resolve_selected_target=self.resolve_selected_target,
            prepare_mapped_session_mirror_output=self.prepare_mapped_session_mirror_output,
            prepare_session_mirror_delegation=self.prepare_session_mirror_delegation,
            channel_typing=self.channel_typing,
            run_steering_prompt=self.run_steering_prompt,
            mark_steering_handoff=self.mark_steering_handoff,
            stream_steering_prompt_result_to_channel=self.stream_steering_prompt_result_to_channel,
            log_line=self.log_line,
            format_log_text_len=self.format_log_text_len,
            monotonic=self.monotonic,
        )

    def make_prefix_status_deps(self) -> status_commands.PrefixStatusCommandDeps:
        return status_commands.PrefixStatusCommandDeps(
            send_chunks=self.status_send_chunks,
            build_where_message=self.build_where_message,
            build_context_message=self.build_context_message,
            build_context_refresh_message=self.build_context_refresh_message,
            clamp_context_refresh_limit=self.clamp_context_refresh_limit,
            build_weekly_usage_message=self.build_weekly_usage_message,
            build_runners_message=self.build_runners_message,
            build_system_resources_message=self.build_system_resources_message,
        )

    def make_prefix_queue_deps(self) -> queue_commands.PrefixQueueCommandDeps:
        return queue_commands.PrefixQueueCommandDeps(
            send_chunks=self.queue_send_chunks,
            retract_queued_ask_for_request=self.retract_queued_ask_for_request,
            log_line=self.log_line,
        )

    def make_prefix_archive_deps(self) -> archive_commands.PrefixArchiveCommandDeps:
        return archive_commands.PrefixArchiveCommandDeps(
            send_chunks=self.archive_send_chunks,
            run_bridge_command=self.run_bridge_command,
        )

    def make_prefix_approval_deps(self) -> approval_commands.PrefixApprovalCommandDeps:
        return approval_commands.PrefixApprovalCommandDeps(
            send_chunks=self.approval_send_chunks,
            get_mirrored_codex_thread_id=self.get_mirrored_codex_thread_id,
            resolve_selected_target=self.resolve_selected_target,
            get_interactive_state_for_thread=self.get_interactive_state_for_thread,
            build_where_message=self.build_where_message,
            send_interactive_prompt=self.send_interactive_prompt,
            interactive_state_approval=self.interactive_state_approval,
        )

    def make_prefix_qa_deps(self) -> qa_command.PrefixQaCommandDeps[BotT]:
        return qa_command.PrefixQaCommandDeps(
            send_chunks=self.qa_send_chunks,
            qa_commands_enabled=self.qa_commands_enabled,
            run_discord_button_qa=self.run_discord_button_qa,
            log_line=self.log_line,
        )

    def make_prefix_new_deps(self) -> new_command.PrefixNewCommandDeps:
        return new_command.PrefixNewCommandDeps(
            send_chunks=self.new_send_chunks,
            run_discord_new_thread=self.run_discord_new_thread,
        )

    def make_prefix_host_deps(self) -> host_commands.PrefixHostCommandDeps:
        return host_commands.PrefixHostCommandDeps(
            send_chunks=self.host_send_chunks,
            host_commands_enabled=self.host_commands_enabled,
            host_reboot_allowed_user_ids_configured=self.host_reboot_allowed_user_ids_configured,
            request_host_reboot=host_commands.request_windows_reboot,
            log_line=self.log_line,
        )
