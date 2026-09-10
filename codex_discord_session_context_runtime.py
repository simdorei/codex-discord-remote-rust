from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import cast

import codex_discord_bridge_protocols as discord_bridge_protocols
import codex_discord_context_refresh as discord_context_refresh
import codex_discord_context_refresh_limits as discord_context_refresh_limits
import codex_discord_origin_prompts as discord_origin_prompts
import codex_discord_recent_user_prompt as discord_recent_user_prompt
import codex_discord_runtime as discord_runtime
import codex_discord_session_mirror as discord_session_mirror
import codex_discord_session_mirror_item_events as discord_session_mirror_item_events
from codex_discord_session_mirror_detail import SessionMirrorDetailMode
import codex_discord_text_digest as discord_text_digest
from codex_session_events import JsonEvent, JsonValue
from codex_app_server_transport_goal import (
    GoalPresent,
    GoalTransportError,
    ThreadGoalLookup,
    ThreadGoalUpdate,
)
from codex_app_server_transport_replies import CodexAppServerTransportError
from codex_app_server_transport_turn_outcomes import TurnCompletion

GetRuntimeStateFunc = Callable[[], discord_runtime.DiscordRuntimeState]
GetContextRefreshBridgeFunc = Callable[[], discord_bridge_protocols.CodexBridgeContextRefreshModule]
BuildInteractiveNoticeFunc = Callable[[Mapping[str, JsonValue]], str]
ExtractMessageTextFunc = Callable[[Mapping[str, JsonValue]], str]
ShouldSkipDiscordOriginPromptFunc = Callable[[str | None, str], bool]
NowFunc = Callable[[], float]
GetThreadGoalLookupFunc = Callable[[str], ThreadGoalLookup]
GetThreadGoalUpdateFunc = Callable[[str, str], ThreadGoalUpdate | None]
GetThreadTurnCompletionsFunc = Callable[[str, list[str]], dict[str, TurnCompletion]]
GetSessionMirrorDetailModeFunc = Callable[[str], SessionMirrorDetailMode]


@dataclass(frozen=True, slots=True)
class SessionContextRuntime:
    get_runtime_state: GetRuntimeStateFunc
    get_context_refresh_bridge: GetContextRefreshBridgeFunc
    make_session_mirror_item_func: discord_context_refresh.MakeSessionMirrorItemFunc
    build_interactive_notice_func: BuildInteractiveNoticeFunc
    extract_message_text_func: ExtractMessageTextFunc
    extract_user_text_func: discord_recent_user_prompt.ExtractUserTextFunc
    iter_recent_session_tail_events_func: discord_recent_user_prompt.IterRecentSessionTailEventsFunc
    should_skip_discord_origin_prompt_func: ShouldSkipDiscordOriginPromptFunc
    now: NowFunc
    ttl_seconds: float
    recent_prompt_dedupe_seconds: float
    recent_prompt_scan_bytes: int
    context_refresh_default_limit: int
    context_refresh_max_limit: int
    context_refresh_item_max_chars: int
    session_mirror_recent_text_ttl_seconds: float
    recent_prompt_expected_exceptions: discord_recent_user_prompt.ExceptionTypes
    format_exception: discord_recent_user_prompt.FormatExceptionFunc
    log: discord_recent_user_prompt.LogFunc
    get_thread_goal_lookup_func: GetThreadGoalLookupFunc
    get_thread_goal_update_func: GetThreadGoalUpdateFunc
    get_thread_turn_completions_func: GetThreadTurnCompletionsFunc
    get_session_mirror_detail_mode_func: GetSessionMirrorDetailModeFunc

    def cleanup_recent_discord_origin_prompts(self, now: float | None = None) -> None:
        current = self.now() if now is None else now
        discord_origin_prompts.cleanup_recent_discord_origin_prompts(
            self.get_runtime_state().recent_discord_origin_prompts,
            ttl_seconds=self.ttl_seconds,
            now=current,
        )

    def mark_recent_discord_origin_prompt(self, target_thread_id: str | None, prompt: str) -> None:
        discord_origin_prompts.mark_recent_discord_origin_prompt(
            self.get_runtime_state().recent_discord_origin_prompts,
            target_thread_id,
            prompt,
            ttl_seconds=self.ttl_seconds,
            now=self.now(),
        )

    def should_skip_discord_origin_prompt(self, target_thread_id: str | None, text: str) -> bool:
        return discord_origin_prompts.should_skip_discord_origin_prompt(
            self.get_runtime_state().recent_discord_origin_prompts,
            target_thread_id,
            text,
            ttl_seconds=self.ttl_seconds,
            now=self.now(),
        )

    def build_interactive_notice_from_payload(self, payload: Mapping[str, JsonValue]) -> str:
        return self.get_context_refresh_bridge().build_interactive_notice_from_function_call(dict(payload))

    def extract_message_text_from_payload(self, payload: Mapping[str, JsonValue]) -> str:
        return self.get_context_refresh_bridge().extract_message_text(dict(payload))

    def extract_user_text_from_session_event(self, event: JsonEvent) -> str:
        return discord_context_refresh.extract_user_text_from_session_event(
            event,
            extract_message_text_func=self.extract_message_text_func,
        )

    def iter_recent_session_tail_events(
        self,
        session_path: Path,
        *,
        scan_bytes: int | None = None,
    ) -> list[JsonEvent]:
        return discord_context_refresh.iter_recent_session_tail_events(
            session_path,
            scan_bytes=self.recent_prompt_scan_bytes if scan_bytes is None else scan_bytes,
        )

    def clamp_context_refresh_limit(self, value: str | int | None) -> int:
        return discord_context_refresh_limits.clamp_context_refresh_limit(
            None if value is None else str(value),
            default=self.context_refresh_default_limit,
            minimum=1,
            maximum=self.context_refresh_max_limit,
        )

    def collect_context_refresh_items(self, codex_thread_id: str, events: list[JsonEvent]) -> list[dict[str, str]]:
        return discord_context_refresh.collect_context_refresh_items(
            codex_thread_id,
            events,
            make_session_mirror_item_func=self.make_session_mirror_item_func,
            build_interactive_notice_func=self.build_interactive_notice_func,
            extract_message_text_func=self.extract_message_text_func,
            make_text_digest_func=discord_text_digest.make_text_digest,
        )

    def format_context_refresh_item(self, item: dict[str, str]) -> str:
        return discord_context_refresh.format_context_refresh_item(
            item,
            max_chars=self.context_refresh_item_max_chars,
        )

    def has_recent_codex_app_user_prompt(
        self,
        target_thread_id: str | None,
        prompt: str,
        *,
        max_age_seconds: float | None = None,
    ) -> bool:
        bridge = self.get_context_refresh_bridge()
        return discord_recent_user_prompt.has_recent_codex_app_user_prompt(
            target_thread_id,
            prompt,
            max_age_seconds=self.recent_prompt_dedupe_seconds if max_age_seconds is None else max_age_seconds,
            deps=discord_recent_user_prompt.RecentCodexAppUserPromptDeps(
                choose_thread=cast(discord_recent_user_prompt.ChooseThreadFunc, bridge.choose_thread),
                iter_recent_session_tail_events=self.iter_recent_session_tail_events_func,
                normalize_prompt_text=bridge.normalize_prompt_text,
                extract_user_text=self.extract_user_text_func,
                parse_timestamp=discord_context_refresh.parse_session_event_timestamp,
                expected_exceptions=self.recent_prompt_expected_exceptions,
                format_exception=self.format_exception,
                log=self.log,
            ),
        )

    def make_session_mirror_item(
        self,
        codex_thread_id: str,
        event: JsonEvent,
        *,
        kind: str,
        role: str,
        phase: str,
        text: str,
    ) -> dict[str, str]:
        return discord_session_mirror.make_session_mirror_item(
            codex_thread_id,
            event,
            kind=kind,
            role=role,
            phase=phase,
            text=text,
            make_text_digest_func=discord_text_digest.make_text_digest,
        )

    def collect_session_mirror_items(
        self,
        codex_thread_id: str,
        events: list[JsonEvent],
        *,
        seen_agent_messages: dict[str, float],
        seen_user_messages: dict[str, float],
    ) -> list[dict[str, str]]:
        turn_ids = discord_session_mirror_item_events.terminal_turn_ids(events)
        turn_completions: dict[str, TurnCompletion] = {}
        turn_completion_error = ""
        if turn_ids:
            try:
                turn_completions = self.get_thread_turn_completions_func(codex_thread_id, turn_ids)
            except (CodexAppServerTransportError, OSError, TimeoutError) as exc:
                turn_completion_error = f"{type(exc).__name__}: {str(exc)[:300]}"

        goal_lookup_error = ""
        goal_status = None
        goal_updates: dict[str, ThreadGoalUpdate] = {}
        if discord_session_mirror_item_events.contains_task_complete_event(events):
            goal_lookup = self.get_thread_goal_lookup_func(codex_thread_id)
            if isinstance(goal_lookup, GoalPresent):
                goal_status = goal_lookup.status
            elif isinstance(goal_lookup, GoalTransportError):
                goal_lookup_error = goal_lookup.message
            for turn_id in turn_ids:
                update = self.get_thread_goal_update_func(codex_thread_id, turn_id)
                if update is not None:
                    goal_updates[turn_id] = update
        return discord_session_mirror.collect_session_mirror_items(
            codex_thread_id,
            events,
            seen_agent_messages=seen_agent_messages,
            seen_user_messages=seen_user_messages,
            should_skip_discord_origin_prompt_func=self.should_skip_discord_origin_prompt_func,
            build_interactive_notice_func=self.build_interactive_notice_func,
            extract_message_text_func=self.extract_message_text_func,
            recent_text_ttl_seconds=self.session_mirror_recent_text_ttl_seconds,
            make_text_digest_func=discord_text_digest.make_text_digest,
            goal_status=goal_status,
            turn_completions=turn_completions,
            turn_completion_error=turn_completion_error,
            goal_updates=goal_updates,
            goal_lookup_error=goal_lookup_error,
            detail_mode=self.get_session_mirror_detail_mode_func(codex_thread_id),
        )
