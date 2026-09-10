import asyncio as _asyncio
from typing import TypeAlias as _TypeAlias
from typing import override as _override

import discord as _discord
import codex_discord_mirror_status as _discord_mirror_status
import codex_discord_stream as _discord_stream
from codex_discord_approval_view import ApprovalView as _ApprovalView
from codex_discord_bot_busy_interaction_adapter_runtime import BotBusyInteractionAdapterRuntime as _BotBusyInteractionAdapterRuntime
from codex_discord_bot_button_qa_adapter_runtime import BotButtonQaAdapterRuntime as _BotButtonQaAdapterRuntime
from codex_discord_bot_context_adapter_runtime import BotContextAdapterRuntime as _BotContextAdapterRuntime
from codex_discord_bot_diagnostics_adapter_runtime import BotDiagnosticsAdapterRuntime as _BotDiagnosticsAdapterRuntime
from codex_discord_bot_history_runtime import BotHistoryRuntime as _BotHistoryRuntime
from codex_discord_bot_interaction_delivery_runtime import BotInteractionDeliveryRuntime as _BotInteractionDeliveryRuntime
from codex_discord_bot_interactive_adapter_runtime import InteractiveChannel as _InteractiveChannel
from codex_discord_bot_interactive_adapter_runtime import SendResult as _InteractiveSendResult
from codex_discord_bot_interactive_adapter_runtime import WatchResult as _InteractiveWatchResult
from codex_discord_bot_interactive_runtime import BotInteractiveRuntime as _BotInteractiveRuntime
from codex_discord_bot_message_adapter_runtime import BotMessageAdapterRuntime as _BotMessageAdapterRuntime
from codex_discord_bot_message_runtime import BotMessageRuntime as _BotMessageRuntime
from codex_discord_bot_misc_adapter_runtime import BotMiscAdapterRuntime as _BotMiscAdapterRuntime
from codex_discord_bot_new_thread_adapter_runtime import BotNewThreadAdapterRuntime as _BotNewThreadAdapterRuntime
from codex_discord_bot_persistent_busy_component_runtime import BotPersistentBusyComponentRuntime as _BotPersistentBusyComponentRuntime
from codex_discord_bot_persistent_component_runtime import BotPersistentComponentRuntime as _BotPersistentComponentRuntime
from codex_discord_bot_plain_ask_adapter_types import BusyChoiceMessage as _BusyChoiceMessage
from codex_discord_bot_plain_ask_adapter_types import BusyChoiceViewValue as _BusyChoiceViewValue
from codex_discord_bot_plain_ask_adapter_types import MessageableChannel as _MessageableChannel
from codex_discord_bot_plain_ask_adapter_types import SendResult as _PlainAskSendResult
from codex_discord_bot_plain_ask_busy_view_runtime import BotPlainAskBusyViewRuntime as _BotPlainAskBusyViewRuntime
from codex_discord_bot_plain_ask_runtime import BotPlainAskRuntime as _BotPlainAskRuntime
from codex_discord_bot_prompt_delivery_adapter_types import PromptChannel as _PromptChannel
from codex_discord_bot_prompt_delivery_adapter_types import PromptRelay as _PromptRelay
from codex_discord_bot_prompt_delivery_adapter_types import SendResult as _PromptSendResult
from codex_discord_bot_prompt_delivery_runtime import BotPromptDeliveryRuntime as _BotPromptDeliveryRuntime
from codex_discord_bot_prompt_transport_adapter_runtime import PromptChannel as _TransportChannel
from codex_discord_bot_prompt_transport_adapter_runtime import SteeringResult as _TransportSteeringResult
from codex_discord_bot_prompt_transport_runtime import BotPromptTransportRuntime as _BotPromptTransportRuntime
from codex_discord_bot_prompt_watch_approval_adapter_runtime import BotPromptWatchApprovalAdapterRuntime as _BotPromptWatchApprovalAdapterRuntime
from codex_discord_bot_session_mirror_delegation_runtime import BotSessionMirrorDelegationRuntime as _BotSessionMirrorDelegationRuntime
from codex_discord_bot_session_mirror_runtime import SessionMirrorTargetMapping as _SessionMirrorTargetMapping
from codex_discord_bot_skill_slash_adapter_runtime import BotSkillSlashAdapterRuntime as _BotSkillSlashAdapterRuntime
from codex_discord_bot_shapes import DiscordMessageableChannel as _DiscordMessageableChannel
from codex_discord_bot_socket_runtime import SocketEventData as _SocketEventData
from codex_discord_bot_socket_runtime import SocketEventPayload as _SocketEventPayload
from codex_discord_bot_stale_busy_adapter_runtime import BotStaleBusyAdapterRuntime as _BotStaleBusyAdapterRuntime
from codex_discord_bot_steering_ack_runtime import BotSteeringAckRuntime as _BotSteeringAckRuntime
from codex_discord_bridge_command_runtime import BridgeCommandRuntime as _BridgeCommandRuntime
from codex_discord_busy_choice_view import BusyChoiceView as _BusyChoiceView
from codex_discord_busy_choice_view import BusyChoiceViewMessage as _BusyChoiceViewMessage
from codex_discord_busy_component_runtime import BusyComponentRuntime as _BusyComponentRuntime
from codex_discord_delivery_runtime import DiscordDeliveryRuntime as _DiscordDeliveryRuntime
from codex_discord_channel_gate import MessageChannelLike as _MessageChannelLike
from codex_discord_interaction_channel_runtime import InteractionChannelRuntime as _InteractionChannelRuntime
from codex_discord_interaction_component_runtime import InteractionComponentRuntime as _InteractionComponentRuntime
from codex_discord_input_choice_view import InputChoiceView as _InputChoiceView
from codex_discord_logging import get_log_path as get_log_path
from codex_discord_message_payload_runtime import MessagePayloadRuntime as _MessagePayloadRuntime
from codex_discord_mirror_access import MirrorAccessBot as _MirrorAccessBot
from codex_discord_mirror_runtime import MirrorRuntime as _MirrorRuntime
from codex_discord_mirror_stale import delete_stale_project_channels as delete_stale_project_channels
from codex_discord_new_thread_flow import format_discord_new_thread_prefix as format_discord_new_thread_prefix
from codex_discord_processed_message_runtime import ProcessedMessageRuntime as _ProcessedMessageRuntime
from codex_discord_project_runtime import ProjectRuntime as _ProjectRuntime
from codex_discord_prompt_bridge_runtime import PromptBridgeRuntime as _PromptBridgeRuntime
from codex_discord_prompt_busy_result import RecentOffsets as _RecentOffsets
from codex_discord_prompt_watch_runtime import PromptWatchRuntime as _PromptWatchRuntime
from codex_discord_ready_runtime import StartupProbeTargetRuntime as _StartupProbeTargetRuntime
from codex_discord_runner_runtime import RunnerRuntime as _RunnerRuntime
from codex_discord_runtime_config_accessors import RuntimeConfigAccessors as _RuntimeConfigAccessors
from codex_discord_runtime_state_bridge import RuntimeStateBridge as _RuntimeStateBridge
from codex_discord_session_context_runtime import SessionContextRuntime as _SessionContextRuntime
from codex_discord_session_mirror_item_delivery import SessionMirrorItem as _SessionMirrorItem
from codex_discord_session_mirror_state_runtime import SessionMirrorStateRuntime as _SessionMirrorStateRuntime
from codex_discord_bot_socket_runtime import BotSocketRuntime as _BotSocketRuntime
from codex_discord_steering import SteeringPromptResult as SteeringPromptResult
from codex_discord_stream_relay import DiscordAskRelay as _DiscordAskRelay
from codex_discord_stream_relay import RelayChannel as _RelayChannel
from codex_discord_text import build_startup_notice as build_startup_notice
from codex_thread_models import ThreadInfo as _ThreadInfo

discord_mirror_status = _discord_mirror_status
discord_stream = _discord_stream

_CachedChannel: _TypeAlias = _discord.abc.GuildChannel | _discord.abc.PrivateChannel | _discord.Thread


class ApprovalView(_ApprovalView):
    def __init__(self, target_thread_id: str) -> None: ...


class InputChoiceView(_InputChoiceView):
    def __init__(self, target_thread_id: str, options: list[tuple[str, str]]) -> None: ...


class BusyChoiceView(_BusyChoiceView):
    def __init__(
        self,
        message: _BusyChoiceViewMessage,
        prompt: str,
        *,
        target_thread_id: str | None = None,
        allow_steer: bool = True,
        choice_id: str | None = None,
    ) -> None: ...


class DiscordAskRelay(_DiscordAskRelay):
    def __init__(
        self,
        loop: _asyncio.AbstractEventLoop,
        channel: _RelayChannel,
        target_thread_id: str | None,
        target_ref: str,
        quiet_notice_delay_sec: float = ...,
        suppress_after_steering_since: float | None = None,
        send_timeout_blocks: bool = True,
        send_commentary_blocks: bool | None = None,
        send_final_blocks: bool = True,
    ) -> None: ...


class LoggingCommandTree:
    async def on_error(
        self,
        interaction: _discord.Interaction[_discord.Client],
        error: BaseException,
        /,
    ) -> None: ...


class CodexDiscordBot(_discord.Client):
    tree: LoggingCommandTree
    allowed_channel_ids: set[int]
    allowed_user_ids: set[int]
    startup_channel_id: int | None
    guild_id: int | None
    enable_prefix_commands: bool
    plain_ask_mention_user_ids: set[int]
    history_poll_seconds: float
    session_mirror_poll_seconds: float
    _history_poll_task: _asyncio.Task[None] | None
    _stop_marker_task: _asyncio.Task[None] | None
    _history_poll_primed_channels: set[int]
    _history_poll_last_at: str
    _session_mirror_task: _asyncio.Task[None] | None
    _session_mirror_last_at: str
    _session_mirror_seen_agent_messages: dict[str, dict[str, float]]
    _session_mirror_seen_user_messages: dict[str, dict[str, float]]
    _session_mirror_archive_skip_logged: set[str]
    _processed_message_ids: dict[str, float]
    _logged_socket_event_ids: dict[str, float]
    _slash_sync_last_at: str
    _slash_sync_status: str
    _slash_sync_commands: str

    def __init__(
        self,
        *,
        allowed_channel_ids: set[int],
        allowed_user_ids: set[int],
        startup_channel_id: int | None,
        guild_id: int | None,
        enable_prefix_commands: bool,
        plain_ask_mention_user_ids: set[int] | None = None,
    ) -> None: ...

    def is_allowed_channel(self, channel_id: int | None) -> bool: ...
    def is_allowed_message_channel(self, channel: _MessageChannelLike) -> bool: ...
    def is_allowed_user(self, user_id: int | None) -> bool: ...

    @_override
    async def setup_hook(self) -> None: ...

    def get_cached_channel_or_thread(self, channel_id: int) -> tuple[_CachedChannel | None, str]: ...
    async def probe_channel_access(self, label: str, channel_id: int) -> None: ...
    async def cleanup_stale_busy_choice_components(self) -> None: ...
    async def log_startup_diagnostics(self) -> None: ...
    async def start_history_polling(self) -> None: ...
    async def history_poll_loop(self) -> None: ...
    async def start_stop_marker_watcher(self) -> None: ...
    async def stop_marker_loop(self) -> None: ...
    async def poll_history_channel(self, label: str, channel_id: int) -> None: ...
    async def process_history_poll_message(self, message: _discord.Message, channel_id: int) -> None: ...
    async def start_session_mirroring(self) -> None: ...
    async def session_mirror_loop(self) -> None: ...
    async def resolve_session_mirror_channel(
        self,
        discord_thread_id: int,
    ) -> object | None: ...
    def get_session_mirror_seen_agent_messages(self, codex_thread_id: str) -> dict[str, float]: ...
    def get_session_mirror_seen_user_messages(self, codex_thread_id: str) -> dict[str, float]: ...
    async def send_session_mirror_item(
        self,
        channel: _DiscordMessageableChannel,
        item: _SessionMirrorItem,
        *,
        target_thread_id: str,
        target_ref: str,
    ) -> None: ...
    async def mirror_session_target(self, target: _SessionMirrorTargetMapping) -> None: ...

    async def on_ready(self) -> None: ...

    async def on_interaction(self, interaction: _discord.Interaction[_discord.Client]) -> None: ...

    async def on_socket_raw_receive(self, message: str | bytes) -> None: ...

    async def on_socket_response(self, payload: _SocketEventPayload) -> None: ...

    def is_tracked_socket_message_channel(self, channel_id: int | None) -> tuple[bool, str]: ...
    async def log_socket_payload(self, payload: _SocketEventData) -> None: ...
    def format_socket_interaction_user(self, data: _SocketEventData) -> str: ...

    async def on_message(self, message: object) -> None: ...

    async def process_discord_message(self, message: object, *, source: str) -> None: ...

_busy_interaction_adapter: _BotBusyInteractionAdapterRuntime
build_busy_choice_message = _busy_interaction_adapter.build_busy_choice_message

_button_qa_adapter: _BotButtonQaAdapterRuntime
run_discord_button_qa = _button_qa_adapter.run_discord_button_qa

_context_adapter: _BotContextAdapterRuntime
build_context_refresh_message = _context_adapter.build_context_refresh_message
build_context_warning = _context_adapter.build_context_warning

_diagnostics_adapter: _BotDiagnosticsAdapterRuntime
build_discord_channel_history_lines = _diagnostics_adapter.build_discord_channel_history_lines
build_discord_doctor_message = _diagnostics_adapter.build_discord_doctor_message
build_discord_tracked_target_user_history_lines = _diagnostics_adapter.build_discord_tracked_target_user_history_lines

_interaction_delivery_runtime: _BotInteractionDeliveryRuntime
run_bridge_and_send = _interaction_delivery_runtime.run_bridge_and_send
run_interaction_bridge_and_send = _interaction_delivery_runtime.run_interaction_bridge_and_send
send_direct_followup = _interaction_delivery_runtime.send_direct_followup
send_followup_chunks = _interaction_delivery_runtime.send_followup_chunks
send_interaction_chunks = _interaction_delivery_runtime.send_interaction_chunks

_interactive_runtime: _BotInteractiveRuntime[_InteractiveChannel, _InteractiveWatchResult, _InteractiveSendResult]
send_interactive_prompt = _interactive_runtime.send_interactive_prompt
submit_interactive_reply = _interactive_runtime.submit_interactive_reply

_message_adapter: _BotMessageAdapterRuntime
handle_prefix_command = _message_adapter.handle_prefix_command

_misc_adapter: _BotMiscAdapterRuntime
main = _misc_adapter.main

_new_thread_adapter: _BotNewThreadAdapterRuntime
handle_slash_new = _new_thread_adapter.handle_slash_new
run_discord_new_thread = _new_thread_adapter.run_discord_new_thread

_persistent_busy_component_runtime: _BotPersistentBusyComponentRuntime
handle_persistent_busy_choice_interaction = _persistent_busy_component_runtime.handle_persistent_busy_choice_interaction

_persistent_component_runtime: _BotPersistentComponentRuntime
handle_persistent_approval_interaction = _persistent_component_runtime.handle_persistent_approval_interaction
handle_persistent_input_choice_interaction = _persistent_component_runtime.handle_persistent_input_choice_interaction
report_unhandled_component_interaction = _persistent_component_runtime.report_unhandled_component_interaction

_plain_ask_busy_view_runtime: _BotPlainAskBusyViewRuntime
make_busy_choice_view = _plain_ask_busy_view_runtime.make_busy_choice_view

PLAIN_ASK_RUNTIME: _BotPlainAskRuntime[
    _MessageableChannel,
    _BusyChoiceMessage,
    _BusyChoiceViewValue,
    _PlainAskSendResult,
]
handle_plain_ask = PLAIN_ASK_RUNTIME.handle_plain_ask
run_prompt_flow = PLAIN_ASK_RUNTIME.run_prompt_flow
send_busy_choice_message = PLAIN_ASK_RUNTIME.send_busy_choice_message

SOCKET_RUNTIME: _BotSocketRuntime
HISTORY_RUNTIME: _BotHistoryRuntime
MESSAGE_RUNTIME: _BotMessageRuntime
PROCESSED_MESSAGE_RUNTIME: _ProcessedMessageRuntime

_prompt_delivery_runtime: _BotPromptDeliveryRuntime[_PromptChannel, _PromptRelay, _PromptSendResult]
handle_recorded_busy_transport_prompt = _prompt_delivery_runtime.handle_recorded_busy_transport_prompt
run_prompt_and_send = _prompt_delivery_runtime.run_prompt_and_send
wait_for_mirrored_busy_delegation_settle = _prompt_delivery_runtime.wait_for_mirrored_busy_delegation_settle

_prompt_transport_runtime: _BotPromptTransportRuntime[
    _TransportChannel,
    _discord_stream.AskStreamRelay,
    _TransportSteeringResult,
]
run_ask_stream = _prompt_transport_runtime.run_ask_stream

_prompt_watch_approval_adapter: _BotPromptWatchApprovalAdapterRuntime
resolve_approval_followup_channel = _prompt_watch_approval_adapter.resolve_approval_followup_channel
stream_post_approval_result_for_interaction = _prompt_watch_approval_adapter.stream_post_approval_result_for_interaction
stream_post_approval_result_to_channel = _prompt_watch_approval_adapter.stream_post_approval_result_to_channel

_session_mirror_delegation_runtime: _BotSessionMirrorDelegationRuntime
prepare_mapped_session_mirror_output = _session_mirror_delegation_runtime.prepare_mapped_session_mirror_output
should_delegate_output_to_session_mirror = _session_mirror_delegation_runtime.should_delegate_output_to_session_mirror

_skill_slash_adapter: _BotSkillSlashAdapterRuntime
handle_slash_ask = _skill_slash_adapter.handle_slash_ask
handle_slash_interview = _skill_slash_adapter.handle_slash_interview

_stale_busy_adapter: _BotStaleBusyAdapterRuntime
get_stale_busy_steer_block_info = _stale_busy_adapter.get_stale_busy_steer_block_info

_steering_ack_runtime: _BotSteeringAckRuntime
send_steering_start_ack = _steering_ack_runtime.send_steering_start_ack

_bridge_command_runtime: _BridgeCommandRuntime
get_busy_state_for_thread = _bridge_command_runtime.get_busy_state_for_thread
get_interactive_state_for_thread = _bridge_command_runtime.get_interactive_state_for_thread
resolve_discord_thread_target_args = _bridge_command_runtime.resolve_discord_thread_target_args
resolve_selected_target = _bridge_command_runtime.resolve_selected_target
resolve_target_ref = _bridge_command_runtime.resolve_target_ref
run_bridge_command = _bridge_command_runtime.run_bridge_command

_busy_component_runtime: _BusyComponentRuntime
claim_busy_choice_record = _busy_component_runtime.claim_busy_choice_record
cleanup_expired_busy_choices = _busy_component_runtime.cleanup_expired_busy_choices
cleanup_expired_persistent_component_claims = _busy_component_runtime.cleanup_expired_persistent_component_claims
clear_stale_busy_choice_message_components = _busy_component_runtime.clear_stale_busy_choice_message_components
get_busy_choice_record = _busy_component_runtime.get_busy_choice_record

_delivery_runtime: _DiscordDeliveryRuntime
clear_discord_delivery_stopping = _delivery_runtime.clear_stopping
send_chunks = _delivery_runtime.send_chunks
send_discord_restarting_notice = _delivery_runtime.send_restarting_notice
send_interaction_not_allowed = _delivery_runtime.send_interaction_not_allowed
send_interaction_response_tracked = _delivery_runtime.send_interaction_response_tracked
send_message_tracked = _delivery_runtime.send_message_tracked
set_discord_delivery_stopping = _delivery_runtime.set_stopping

_interaction_channel_runtime: _InteractionChannelRuntime
get_interaction_gate_command_name = _interaction_channel_runtime.get_interaction_gate_command_name
is_mirrored_interaction_channel_id = _interaction_channel_runtime.is_mirrored_interaction_channel_id

_interaction_component_runtime: _InteractionComponentRuntime
clear_interaction_message_components = _interaction_component_runtime.clear_interaction_message_components
resolve_interaction_channel = _interaction_component_runtime.resolve_interaction_channel

_message_payload_runtime: _MessagePayloadRuntime
build_prompt_with_discord_attachments = _message_payload_runtime.build_prompt_with_discord_attachments
maybe_send_empty_content_notice = _message_payload_runtime.maybe_send_empty_content_notice

_mirror_runtime: _MirrorRuntime[_MirrorAccessBot]
build_mirror_check = _mirror_runtime.build_mirror_check
build_mirror_list = _mirror_runtime.build_mirror_list
mirror_single_codex_thread = _mirror_runtime.mirror_single_codex_thread
refresh_codex_bridge_session_state = _mirror_runtime.refresh_codex_bridge_session_state
refresh_discord_bridge_session = _mirror_runtime.refresh_discord_bridge_session

_processed_message_runtime: _ProcessedMessageRuntime
claim_discord_message = _processed_message_runtime.claim_discord_message
mark_discord_message_processed = _processed_message_runtime.mark_discord_message_processed

_project_runtime: _ProjectRuntime
describe_mirrored_project_channel = _project_runtime.describe_mirrored_project_channel
get_mirrored_codex_thread_id = _project_runtime.get_mirrored_codex_thread_id
normalize_project_key = _project_runtime.normalize_project_key
resolve_discord_new_thread_cwd = _project_runtime.resolve_discord_new_thread_cwd
resolve_discord_new_thread_project_channel_id = _project_runtime.resolve_discord_new_thread_project_channel_id

_prompt_bridge_runtime: _PromptBridgeRuntime
app_server_transport_enabled = _prompt_bridge_runtime.app_server_transport_enabled
get_bridge_script_path = _prompt_bridge_runtime.get_bridge_script_path
run_ask = _prompt_bridge_runtime.run_ask
run_bridge_command_stream = _prompt_bridge_runtime.run_bridge_command_stream
run_legacy_ipc_prompt_no_wait = _prompt_bridge_runtime.run_legacy_ipc_prompt_no_wait
run_resident_app_server_steering_prompt = _prompt_bridge_runtime.run_resident_app_server_steering_prompt
run_steering_prompt = _prompt_bridge_runtime.run_steering_prompt
run_transport_prompt_no_wait = _prompt_bridge_runtime.run_transport_prompt_no_wait
submit_approval_reply = _prompt_bridge_runtime.submit_approval_reply
submit_input_reply = _prompt_bridge_runtime.submit_input_reply

_startup_probe_runtime: _StartupProbeTargetRuntime
get_startup_probe_targets = _startup_probe_runtime.get_startup_probe_targets

_runner_runtime: _RunnerRuntime
enqueue_thread_ask = _runner_runtime.enqueue_thread_ask
get_thread_runner = _runner_runtime.get_thread_runner
resolve_queue_command_target = _runner_runtime.resolve_queue_command_target

_runtime_config_accessors: _RuntimeConfigAccessors
get_ask_busy_retry_delay_seconds = _runtime_config_accessors.get_ask_busy_retry_delay_seconds
get_startup_channel_probe_timeout = _runtime_config_accessors.get_startup_channel_probe_timeout

_session_context_runtime: _SessionContextRuntime
collect_session_mirror_items = _session_context_runtime.collect_session_mirror_items
has_recent_codex_app_user_prompt = _session_context_runtime.has_recent_codex_app_user_prompt
mark_recent_discord_origin_prompt = _session_context_runtime.mark_recent_discord_origin_prompt

_session_mirror_state_runtime: _SessionMirrorStateRuntime
activate_pending_session_mirror_output_target = _session_mirror_state_runtime.activate_pending_session_mirror_output_target
activate_session_mirror_output_target = _session_mirror_state_runtime.activate_session_mirror_output_target
claim_session_mirror_event = _session_mirror_state_runtime.claim_session_mirror_event
get_or_init_session_mirror_cursor = _session_mirror_state_runtime.get_or_init_session_mirror_cursor
has_session_mirror_event = _session_mirror_state_runtime.has_session_mirror_event
is_active_session_mirror_output_target = _session_mirror_state_runtime.is_active_session_mirror_output_target
is_pending_session_mirror_cursor_target = _session_mirror_state_runtime.is_pending_session_mirror_cursor_target
prime_session_mirror_cursor_for_target = _session_mirror_state_runtime.prime_session_mirror_cursor_for_target
session_mirror_rollout_path_missing = _session_mirror_state_runtime.session_mirror_rollout_path_missing
update_session_mirror_cursor = _session_mirror_state_runtime.update_session_mirror_cursor

_runtime_state_bridge: _RuntimeStateBridge
acquire_runtime_instance_lock = _runtime_state_bridge.acquire_runtime_instance_lock
get_runtime_state = _runtime_state_bridge.get_runtime_state
get_session_mirror_state = _runtime_state_bridge.get_session_mirror_state
is_thread_runner_busy = _runtime_state_bridge.is_thread_runner_busy
mark_steering_handoff = _runtime_state_bridge.mark_steering_handoff
remove_runtime_lock_for_current_process = _runtime_state_bridge.remove_runtime_lock_for_current_process

_prompt_watch_runtime: _PromptWatchRuntime
make_approval_followup_relay = _prompt_watch_runtime.make_approval_followup_relay
make_post_approval_watch_result = _prompt_watch_runtime.make_post_approval_watch_result
make_steering_watch_relay = _prompt_watch_runtime.make_steering_watch_relay
run_steering_watch_stream = _prompt_watch_runtime.run_steering_watch_stream
stream_steering_prompt_result_to_channel = _prompt_watch_runtime.stream_steering_prompt_result_to_channel

async def send_context_exhausted_prompt_notice_if_needed(
    channel: _DiscordMessageableChannel,
    target_thread_id: str | None,
    target_ref: str,
) -> bool: ...

def snapshot_ask_prompt_delivery_state(
    target_thread_id: str | None,
) -> tuple[_ThreadInfo | None, _RecentOffsets]: ...
