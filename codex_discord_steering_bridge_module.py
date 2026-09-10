from __future__ import annotations

from collections.abc import Sequence
from typing import Final

import codex_app_server_transport as app_server_transport
import codex_discord_steering as discord_steering
from codex_discord_bridge_modules import BRIDGE_APP_SERVER_DELIVERY
from codex_thread_models import ThreadInfo


class SteeringBridgeThreadTypeError(TypeError):
    def __init__(self, thread: discord_steering.SteeringThreadLike) -> None:
        self.thread: discord_steering.SteeringThreadLike = thread
        super().__init__(f"Expected bridge.ThreadInfo, got {type(thread).__name__}")


def require_steering_thread_info(
    thread: discord_steering.SteeringThreadLike,
) -> ThreadInfo:
    if isinstance(thread, ThreadInfo):
        return thread
    raise SteeringBridgeThreadTypeError(thread)


class CodexSteeringBridge:
    def choose_thread(self, thread_id: str | None, cwd: str | None) -> ThreadInfo:
        return BRIDGE_APP_SERVER_DELIVERY.choose_thread(thread_id, cwd)

    def get_thread_workspace_ref(self, thread: discord_steering.SteeringThreadLike) -> str:
        return BRIDGE_APP_SERVER_DELIVERY.get_thread_workspace_ref(
            require_steering_thread_info(thread)
        )

    def snapshot_recent_session_offsets(
        self,
        *,
        limit: int,
        include_threads: Sequence[discord_steering.SteeringThreadLike] | None,
    ) -> discord_steering.SteeringRecentOffsets:
        thread_infos = (
            [require_steering_thread_info(thread) for thread in include_threads]
            if include_threads is not None
            else None
        )
        return BRIDGE_APP_SERVER_DELIVERY.snapshot_recent_session_offsets(
            limit=limit,
            include_threads=thread_infos,
        )

    def wait_for_prompt_delivery(
        self,
        recent_offsets: discord_steering.SteeringRecentOffsets,
        prompt: str,
        *,
        timeout_sec: float,
    ) -> ThreadInfo | None:
        session_offsets = {
            thread_id: (require_steering_thread_info(thread), session_path, start_offset)
            for thread_id, (thread, session_path, start_offset) in recent_offsets.items()
        }
        return BRIDGE_APP_SERVER_DELIVERY.wait_for_prompt_delivery(
            session_offsets,
            prompt,
            timeout_sec=timeout_sec,
        )

    def get_thread_label(self, thread: discord_steering.SteeringThreadLike) -> str:
        return BRIDGE_APP_SERVER_DELIVERY.get_thread_label(
            require_steering_thread_info(thread)
        )


STEERING_BRIDGE_MODULE: Final[discord_steering.SteeringBridgeLike] = CodexSteeringBridge()


def get_steering_bridge_module() -> discord_steering.SteeringBridgeLike:
    return STEERING_BRIDGE_MODULE


def make_app_server_steering_result(
    result: app_server_transport.AppServerDeliveryResult,
) -> discord_steering.SteeringPromptResult:
    watch_target: discord_steering.WatchTarget = (
        discord_steering.NativeExactWatchTarget(result.turn_id)
        if result.turn_id
        else discord_steering.RolloutOnlyWatchTarget()
    )
    return discord_steering.SteeringPromptResult(
        result.exit_code,
        result.output,
        target_thread_id=result.thread_id,
        target_ref=result.target_ref,
        session_path=result.session_path,
        start_offset=result.start_offset,
        delivery_pending=result.delivery_pending,
        watch_target=watch_target,
    )
