from __future__ import annotations

from contextlib import AbstractContextManager
from typing import Protocol

import codex_desktop_bridge as bridge
from codex_app_server_transport_delivery_results import (
    AppServerDeliveryResult,
    BridgeModule,
    build_delivery_context,
    cross_thread_delivery_result,
    successful_delivery_result,
    wait_for_delivery,
)
from codex_app_server_transport_replies import JsonObject
from codex_app_server_transport_threads import ensure_thread_loaded, result_turn_id

__all__ = [
    "AppServerDeliveryClient",
    "AppServerDeliveryResult",
    "BridgeModule",
    "start_turn_no_wait",
    "steer_or_start_no_wait",
]


class AppServerDeliveryClient(Protocol):
    def read_thread(
        self,
        thread_id: str,
        *,
        include_turns: bool = False,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def resume_thread(
        self,
        thread_id: str,
        *,
        timeout_sec: float = 10.0,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def start_turn(
        self,
        thread_id: str,
        prompt: str,
        *,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def steer_turn(
        self,
        thread_id: str,
        prompt: str,
        *,
        expected_turn_id: str,
        expected_generation: int | None = None,
    ) -> JsonObject: ...
    def get_active_turn_id(
        self,
        thread_id: str,
        *,
        expected_generation: int | None = None,
    ) -> str | None: ...
    def is_thread_subscribed(self, thread_id: str) -> bool: ...
    def thread_subscription_lock(self, thread_id: str) -> AbstractContextManager[None]: ...
    def note_thread_activity(self, thread_id: str) -> None: ...


_DEFAULT_BRIDGE_MODULE: BridgeModule = bridge


def start_turn_no_wait(
    client: AppServerDeliveryClient,
    prompt: str,
    target_thread_id: str | None,
    *,
    bridge_module: BridgeModule = _DEFAULT_BRIDGE_MODULE,
    confirm_timeout_sec: float = 6.0,
    expected_generation: int | None = None,
) -> AppServerDeliveryResult:
    context = build_delivery_context(
        target_thread_id,
        bridge_module=bridge_module,
    )
    thread = context.thread
    with client.thread_subscription_lock(thread.id):
        client.note_thread_activity(thread.id)
        _ = ensure_thread_loaded(client, thread.id, expected_generation=expected_generation)
        if expected_generation is None:
            result = client.start_turn(thread.id, prompt)
        else:
            result = client.start_turn(thread.id, prompt, expected_generation=expected_generation)
    turn_id = result_turn_id(result)
    delivered_thread = wait_for_delivery(
        context,
        prompt,
        bridge_module=bridge_module,
        timeout_sec=confirm_timeout_sec,
    )
    if delivered_thread is not None and delivered_thread.id != thread.id:
        return cross_thread_delivery_result(
            context,
            delivered_thread=delivered_thread,
            turn_id=turn_id,
            bridge_module=bridge_module,
        )
    delivery_pending = delivered_thread is None
    return successful_delivery_result(
        context,
        method="turn/start",
        turn_id=turn_id,
        delivered_thread=delivered_thread,
        delivery_pending=delivery_pending,
        bridge_module=bridge_module,
    )


def steer_or_start_no_wait(
    client: AppServerDeliveryClient,
    prompt: str,
    target_thread_id: str | None,
    *,
    bridge_module: BridgeModule = _DEFAULT_BRIDGE_MODULE,
    confirm_timeout_sec: float = 6.0,
    expected_generation: int | None = None,
) -> AppServerDeliveryResult:
    context = build_delivery_context(
        target_thread_id,
        bridge_module=bridge_module,
    )
    thread = context.thread
    with client.thread_subscription_lock(thread.id):
        client.note_thread_activity(thread.id)
        _ = ensure_thread_loaded(client, thread.id, expected_generation=expected_generation)
        if expected_generation is None:
            active_turn_id = client.get_active_turn_id(thread.id)
        else:
            active_turn_id = client.get_active_turn_id(thread.id, expected_generation=expected_generation)
        method = "turn/steer" if active_turn_id else "turn/start"
        if active_turn_id:
            if expected_generation is None:
                result = client.steer_turn(thread.id, prompt, expected_turn_id=active_turn_id)
            else:
                result = client.steer_turn(
                    thread.id,
                    prompt,
                    expected_turn_id=active_turn_id,
                    expected_generation=expected_generation,
                )
            turn_id = result_turn_id(result, fallback=active_turn_id)
        else:
            if expected_generation is None:
                result = client.start_turn(thread.id, prompt)
            else:
                result = client.start_turn(thread.id, prompt, expected_generation=expected_generation)
            turn_id = result_turn_id(result)
    delivered_thread = wait_for_delivery(
        context,
        prompt,
        bridge_module=bridge_module,
        timeout_sec=confirm_timeout_sec,
    )
    if delivered_thread is not None and delivered_thread.id != thread.id:
        return cross_thread_delivery_result(
            context,
            delivered_thread=delivered_thread,
            turn_id=turn_id,
            bridge_module=bridge_module,
        )
    delivery_pending = delivered_thread is None
    return successful_delivery_result(
        context,
        method=method,
        turn_id=turn_id,
        delivered_thread=delivered_thread,
        delivery_pending=delivery_pending,
        bridge_module=bridge_module,
    )
