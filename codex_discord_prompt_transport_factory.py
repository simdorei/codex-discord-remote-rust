from __future__ import annotations

from contextlib import AbstractContextManager
from typing import assert_never, Final, TypeAlias, TypeVar

import codex_app_server_transport as app_server_transport
import codex_app_server_transport_delivery as app_server_delivery
from codex_app_server_transport_turn_outcomes import (
    TurnCompletionFound,
    TurnCompletionPending,
    TurnCompletionTransportError,
    TurnStatus,
)
import codex_discord_app_server as discord_app_server
import codex_discord_app_server_admission as discord_app_server_admission
import codex_discord_prompt_transport as prompt_transport
import codex_discord_runtime_config as runtime_config
import codex_discord_stream as discord_stream
import codex_discord_ui_ask as discord_ui_ask
import codex_pro_browser_evidence as pro_browser_evidence
import codex_pro_connector_evidence as pro_connector_evidence


RelayT = TypeVar("RelayT", bound=discord_stream.AskStreamRelay)
SteeringResultT = TypeVar("SteeringResultT")
AppServerDeliveryResult: TypeAlias = app_server_transport.AppServerDeliveryResult
AppServerStartTurnNoWait: TypeAlias = prompt_transport.StartTurnNoWait[AppServerDeliveryResult]
DEFAULT_APP_SERVER_DELIVERY_CONFIRM_TIMEOUT_SECONDS: Final = 25.0
PRO_TURN_COMPLETION_TIMEOUT_SECONDS: Final = 7200.0


class ProMappedThreadRequiredError(RuntimeError):
    def __init__(self) -> None:
        super().__init__("Pro Chrome requires a mapped Codex thread.")


class ProDeliveryIdentityMissingError(RuntimeError):
    def __init__(self) -> None:
        super().__init__(
            "Pro app-server delivery returned no exact thread and turn identity."
        )


class ProTurnCompletionError(RuntimeError):
    def __init__(self, detail: str) -> None:
        self.detail: str = detail
        super().__init__(f"Pro app-server turn did not complete successfully: {detail}")


def _wait_for_pro_turn_completion(target_thread_id: str, turn_id: str) -> None:
    observation = app_server_transport.DEFAULT_CLIENT.wait_for_turn_completion(
        target_thread_id,
        turn_id,
        timeout_sec=PRO_TURN_COMPLETION_TIMEOUT_SECONDS,
    )
    match observation:  # noqa: MATCH_OK - exhaustive cases end in assert_never below.
        case TurnCompletionFound(completion=completion):
            match completion.status:  # noqa: MATCH_OK - exhaustive cases end in assert_never below.
                case TurnStatus.COMPLETED:
                    return
                case TurnStatus.FAILED | TurnStatus.INTERRUPTED | TurnStatus.IN_PROGRESS:
                    detail = completion.error_message or completion.status.value
                    raise ProTurnCompletionError(detail)
            assert_never(completion.status)
        case TurnCompletionPending():
            raise TimeoutError("Timed out waiting for the Pro app-server turn to complete.")
        case TurnCompletionTransportError(message=message):
            raise ProTurnCompletionError(message)
    assert_never(observation)


def get_app_server_delivery_confirm_timeout() -> float:
    return runtime_config.get_steering_delivery_confirm_timeout(
        default=DEFAULT_APP_SERVER_DELIVERY_CONFIRM_TIMEOUT_SECONDS,
    )


def make_prompt_transport_deps(
    *,
    bridge_module: app_server_delivery.BridgeModule,
    app_server_transport_enabled: prompt_transport.TransportEnabled,
    run_legacy_prompt_no_wait: prompt_transport.PromptNoWait,
    make_steering_prompt_result: prompt_transport.MakeSteeringResult[
        AppServerDeliveryResult,
        SteeringResultT,
    ],
    run_watch_stream: prompt_transport.WatchStream[SteeringResultT, RelayT],
    run_bridge_command_stream: discord_stream.RunBridgeCommandStreamFunc,
    ui_fallback_lock: AbstractContextManager[bool],
    log: prompt_transport.LogFunc,
    run_resident_prompt_no_wait: prompt_transport.PromptNoWait | None = None,
    start_turn_no_wait: AppServerStartTurnNoWait | None = None,
    complete_pro_browser_session: prompt_transport.CompleteProBrowserSession | None = None,
) -> prompt_transport.PromptTransportDeps[RelayT, AppServerDeliveryResult, SteeringResultT]:
    def prepare_pro_browser_session(target_thread_id: str | None) -> None:
        if not target_thread_id:
            raise ProMappedThreadRequiredError
        log(f"pro_browser_session_admitted target={target_thread_id}")

    def complete_pro_browser_session_impl(
        target_thread_id: str | None,
        turn_id: str | None,
    ) -> None:
        if complete_pro_browser_session is not None:
            complete_pro_browser_session(target_thread_id, turn_id)
            return
        if not target_thread_id or not turn_id:
            raise ProDeliveryIdentityMissingError
        _wait_for_pro_turn_completion(target_thread_id, turn_id)
        pro_browser_evidence.require_available_evidence(target_thread_id, turn_id)
        pro_connector_evidence.require_verified_evidence(target_thread_id, turn_id)
        log(f"pro_connector_verified target={target_thread_id} turn={turn_id}")
        log(f"pro_browser_session_verified target={target_thread_id} turn={turn_id}")

    def run_resident_prompt_no_wait_impl(prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        if run_resident_prompt_no_wait is not None:
            return run_resident_prompt_no_wait(prompt, target_thread_id)
        return discord_app_server.run_prompt_no_wait(
            prompt,
            target_thread_id,
            transport_module=app_server_transport,
            bridge_module=bridge_module,
            client=app_server_transport.DEFAULT_CLIENT,
            confirm_timeout_sec=get_app_server_delivery_confirm_timeout(),
        )

    def start_turn_no_wait_impl(prompt: str, target_thread_id: str | None) -> AppServerDeliveryResult:
        if start_turn_no_wait is not None:
            return start_turn_no_wait(prompt, target_thread_id)
        expected_generation = discord_app_server_admission.current_expected_app_server_generation()
        if expected_generation is None:
            return app_server_transport.steer_or_start_no_wait(
                app_server_transport.DEFAULT_CLIENT,
                prompt,
                target_thread_id,
                bridge_module=bridge_module,
                confirm_timeout_sec=get_app_server_delivery_confirm_timeout(),
            )
        return app_server_transport.steer_or_start_no_wait(
            app_server_transport.DEFAULT_CLIENT,
            prompt,
            target_thread_id,
            bridge_module=bridge_module,
            confirm_timeout_sec=get_app_server_delivery_confirm_timeout(),
            expected_generation=expected_generation,
        )

    def run_legacy_stream(
        prompt: str,
        relay: RelayT,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
    ) -> tuple[int, str]:
        return discord_stream.run_ask_stream(
            prompt,
            relay,
            force_while_busy=force_while_busy,
            wait=wait,
            target_thread_id=target_thread_id,
            use_sidecar=False,
            no_fallback=True,
            allow_ui_fallback=False,
            run_bridge_command_stream_func=run_bridge_command_stream,
            should_retry_ask_with_ui_func=discord_ui_ask.should_retry_ask_with_ui,
            build_ui_ask_argv_func=discord_ui_ask.build_ui_ask_argv,
            ui_fallback_lock=ui_fallback_lock,
        )

    return prompt_transport.PromptTransportDeps(
        app_server_transport_enabled=app_server_transport_enabled,
        prepare_pro_browser_session=prepare_pro_browser_session,
        complete_pro_browser_session=complete_pro_browser_session_impl,
        run_resident_prompt_no_wait=run_resident_prompt_no_wait_impl,
        run_legacy_prompt_no_wait=run_legacy_prompt_no_wait,
        start_turn_no_wait=start_turn_no_wait_impl,
        make_steering_prompt_result=make_steering_prompt_result,
        run_watch_stream=run_watch_stream,
        run_legacy_stream=run_legacy_stream,
        log=log,
    )
