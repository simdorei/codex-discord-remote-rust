from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import TypeAlias

import codex_app_server_transport as app_server_transport
import codex_discord_app_server as discord_app_server
import codex_discord_app_server_bot_bridge as discord_app_server_bot_bridge
import codex_discord_bridge_process as bridge_process
import codex_discord_prompt_transport as discord_prompt_transport
import codex_discord_steering as discord_steering
from codex_app_server_transport_delivery import BridgeModule

EnvFlagFunc: TypeAlias = Callable[[str, bool], bool]
GetPathFunc: TypeAlias = Callable[[], Path]
PromptTransportDepsFunc: TypeAlias = Callable[
    [],
    discord_prompt_transport.PromptTransportDeps[
        discord_prompt_transport.PromptRelay,
        app_server_transport.AppServerDeliveryResult,
        discord_steering.SteeringPromptResult,
    ],
]
GetSteeringBridgeModuleFunc: TypeAlias = Callable[[], discord_steering.SteeringBridgeLike]
GetAppServerClientFunc: TypeAlias = Callable[[], discord_app_server.AppServerClient]


@dataclass(frozen=True, slots=True)
class PromptBridgeRuntime:
    script_dir: Path
    env_flag: EnvFlagFunc
    get_bridge_script_path_func: GetPathFunc
    run_bridge_command_func: discord_app_server_bot_bridge.RunBridgeCommandFunc
    make_prompt_transport_deps_func: PromptTransportDepsFunc
    resolve_target_ref_func: discord_app_server_bot_bridge.ResolveTargetRefFunc
    run_steering_no_wait_func: discord_app_server_bot_bridge.RunSteeringNoWaitFunc
    app_server_transport_module: discord_app_server.AppServerTransportModule
    app_server_bridge_module: BridgeModule
    get_app_server_client_func: GetAppServerClientFunc
    get_steering_delivery_confirm_timeout_func: discord_steering.GetTimeoutFunc
    is_app_server_transport_enabled_func: Callable[[], bool]
    submit_approval_reply_func: discord_app_server_bot_bridge.SubmitReplyFunc
    submit_input_reply_func: discord_app_server_bot_bridge.SubmitReplyFunc
    pending_input_bridge: discord_app_server_bot_bridge.PendingInputReplyBridge
    run_resident_steering_prompt_func: Callable[[str, str | None], discord_steering.SteeringPromptResult]
    run_ask_func: discord_steering.RunAskFunc
    get_steering_bridge_module_func: GetSteeringBridgeModuleFunc
    log: discord_steering.LogFunc
    format_log_text_len: discord_steering.FormatLogTextLenFunc

    def get_bridge_script_path(self) -> Path:
        return bridge_process.get_bridge_script_path(self.script_dir)

    def run_bridge_command_stream(
        self,
        argv: list[str],
        on_line: bridge_process.LineCallback,
    ) -> tuple[int, str]:
        return bridge_process.run_bridge_command_stream(
            argv,
            on_line,
            script_path=self.get_bridge_script_path_func(),
            cwd=self.script_dir,
            env=bridge_process.build_bridge_subprocess_env(),
        )

    def run_ask(
        self,
        prompt: str,
        *,
        force_while_busy: bool = False,
        wait: bool = True,
        target_thread_id: str | None = None,
        timeout_sec: float | None = None,
    ) -> tuple[int, str]:
        timeout_value = "0" if timeout_sec is None else str(max(1, int(timeout_sec)))
        argv = [
            "ask",
            "--ipc",
            "--foreground",
            "--no-fallback",
            "--timeout",
            timeout_value,
        ]
        if target_thread_id:
            argv.extend(["--thread-id", target_thread_id])
        if force_while_busy:
            argv.append("--force-while-busy")
        if not wait:
            argv.append("--no-wait")
        argv.append(prompt)
        return self.run_bridge_command_func(argv)

    def app_server_transport_enabled(self) -> bool:
        return self.env_flag("CODEX_DISCORD_APP_SERVER_TRANSPORT", True)

    def run_legacy_ipc_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        argv = [
            "ask",
            "--ipc",
            "--foreground",
            "--no-fallback",
            "--timeout",
            "0",
            "--no-wait",
        ]
        if target_thread_id:
            argv.extend(["--thread-id", target_thread_id])
        argv.append(prompt)
        return self.run_bridge_command_func(argv)

    def run_transport_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        return discord_prompt_transport.run_transport_prompt_no_wait(
            prompt,
            target_thread_id,
            self.make_prompt_transport_deps_func(),
        )

    def run_resident_app_server_steering_prompt(
        self,
        prompt: str,
        target_thread_id: str | None,
    ) -> discord_steering.SteeringPromptResult:
        return discord_app_server_bot_bridge.run_resident_app_server_steering_prompt(
            prompt,
            target_thread_id,
            resolve_target_ref_func=self.resolve_target_ref_func,
            run_steering_no_wait_func=self.run_steering_no_wait_func,
            transport_module=self.app_server_transport_module,
            bridge_module=self.app_server_bridge_module,
            client=self.get_app_server_client_func(),
            get_confirm_timeout_func=self.get_steering_delivery_confirm_timeout_func,
            expected_exceptions=(OSError, RuntimeError, ValueError),
            log_func=self.log,
        )

    def submit_approval_reply(self, target_thread_id: str, answer: str) -> tuple[int, str]:
        return discord_app_server_bot_bridge.submit_approval_reply(
            target_thread_id,
            answer,
            app_server_transport_enabled_func=self.is_app_server_transport_enabled_func,
            submit_reply_func=self.submit_approval_reply_func,
            client=self.get_app_server_client_func(),
            run_bridge_command_func=self.run_bridge_command_func,
        )

    def submit_input_reply(self, target_thread_id: str, answer: str) -> tuple[int, str]:
        return discord_app_server_bot_bridge.submit_input_reply(
            target_thread_id,
            answer,
            app_server_transport_enabled_func=self.is_app_server_transport_enabled_func,
            submit_reply_func=self.submit_input_reply_func,
            client=self.get_app_server_client_func(),
            pending_input_bridge=self.pending_input_bridge,
        )

    def run_steering_prompt(
        self,
        prompt: str,
        target_thread_id: str | None,
    ) -> discord_steering.SteeringPromptResult:
        if self.is_app_server_transport_enabled_func():
            return self.run_resident_steering_prompt_func(prompt, target_thread_id)
        return discord_steering.run_steering_prompt(
            prompt,
            target_thread_id,
            bridge_module=self.get_steering_bridge_module_func(),
            resolve_target_ref_func=self.resolve_target_ref_func,
            run_ask_func=self.run_ask_func,
            get_steering_delivery_confirm_timeout_func=self.get_steering_delivery_confirm_timeout_func,
            log_func=self.log,
            format_log_text_len_func=self.format_log_text_len,
        )
