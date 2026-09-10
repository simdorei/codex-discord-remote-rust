from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from types import ModuleType
from typing import cast, TypeAlias

import codex_app_server_transport as app_server_transport
import codex_discord_app_server as discord_app_server
import codex_discord_app_server_bot_bridge as discord_app_server_bot_bridge
import codex_discord_prompt_bridge_runtime as discord_prompt_bridge_runtime
import codex_discord_prompt_transport as discord_prompt_transport
import codex_discord_steering as discord_steering
from codex_app_server_transport_delivery import AppServerDeliveryClient, BridgeModule
ModuleValue: TypeAlias = object


@dataclass(frozen=True, slots=True)
class BotPromptBridgeAdapterRuntime:
    module: ModuleType

    def make_prompt_bridge_runtime(self) -> discord_prompt_bridge_runtime.PromptBridgeRuntime:
        return discord_prompt_bridge_runtime.PromptBridgeRuntime(
            script_dir=cast(Path, getattr(self.module, "SCRIPT_DIR")),
            env_flag=cast(discord_prompt_bridge_runtime.EnvFlagFunc, self._module_func("env_flag")),
            get_bridge_script_path_func=self.get_bridge_script_path,
            run_bridge_command_func=self.run_bridge_command,
            make_prompt_transport_deps_func=self.make_prompt_transport_deps,
            resolve_target_ref_func=self.resolve_target_ref,
            run_steering_no_wait_func=self.run_steering_no_wait,
            app_server_transport_module=cast(
                discord_app_server.AppServerTransportModule,
                getattr(self.module, "app_server_transport"),
            ),
            app_server_bridge_module=cast(BridgeModule, getattr(self.module, "BRIDGE_APP_SERVER_DELIVERY")),
            get_app_server_client_func=self.get_app_server_client,
            get_steering_delivery_confirm_timeout_func=self.get_steering_delivery_confirm_timeout,
            is_app_server_transport_enabled_func=self.app_server_transport_enabled,
            submit_approval_reply_func=self.submit_approval_reply_to_app_server,
            submit_input_reply_func=self.submit_input_reply_to_app_server,
            pending_input_bridge=cast(
                discord_app_server_bot_bridge.PendingInputReplyBridge,
                getattr(self.module, "BRIDGE_PENDING_INPUT_REPLY"),
            ),
            run_resident_steering_prompt_func=self.run_resident_app_server_steering_prompt,
            run_ask_func=self.run_ask,
            get_steering_bridge_module_func=self.get_steering_bridge_module,
            log=self.log_line,
            format_log_text_len=self.format_log_text_len,
        )

    def get_bridge_script_path(self) -> Path:
        return cast(Callable[[], Path], self._module_func("get_bridge_script_path"))()

    def run_bridge_command(self, argv: list[str]) -> tuple[int, str]:
        return cast(Callable[[list[str]], tuple[int, str]], self._module_func("run_bridge_command"))(argv)

    def make_prompt_transport_deps(
        self,
    ) -> discord_prompt_transport.PromptTransportDeps[
        discord_prompt_transport.PromptRelay,
        app_server_transport.AppServerDeliveryResult,
        discord_steering.SteeringPromptResult,
    ]:
        return cast(
            discord_prompt_bridge_runtime.PromptTransportDepsFunc,
            self._module_func("make_prompt_transport_deps"),
        )()

    def resolve_target_ref(self, target_thread_id: str | None) -> tuple[str | None, str]:
        return cast(
            discord_app_server_bot_bridge.ResolveTargetRefFunc,
            self._module_func("resolve_target_ref"),
        )(target_thread_id)

    def run_steering_no_wait(
        self,
        prompt: str,
        target_thread_id: str | None,
        *,
        transport_module: discord_app_server.AppServerTransportModule,
        bridge_module: BridgeModule,
        client: AppServerDeliveryClient,
        confirm_timeout_sec: float,
    ) -> discord_app_server_bot_bridge.AppServerSteeringResult:
        return cast(
            discord_app_server_bot_bridge.RunSteeringNoWaitFunc,
            self._module_attr("discord_app_server", "run_steering_no_wait"),
        )(
            prompt,
            target_thread_id,
            transport_module=transport_module,
            bridge_module=bridge_module,
            client=client,
            confirm_timeout_sec=confirm_timeout_sec,
        )

    def get_app_server_client(self) -> discord_app_server.AppServerClient:
        transport_module = cast(discord_app_server.AppServerTransportModule, getattr(self.module, "app_server_transport"))
        return cast(discord_app_server.AppServerClient, transport_module.DEFAULT_CLIENT)

    def get_steering_delivery_confirm_timeout(self) -> float:
        return cast(Callable[[], float], self._module_func("get_steering_delivery_confirm_timeout"))()

    def app_server_transport_enabled(self) -> bool:
        return cast(Callable[[], bool], self._module_func("app_server_transport_enabled"))()

    def submit_approval_reply_to_app_server(
        self,
        target_thread_id: str,
        answer: str,
        *,
        client: discord_app_server.AppServerClient | None = None,
    ) -> tuple[int, str] | None:
        return cast(
            discord_app_server_bot_bridge.SubmitReplyFunc,
            self._module_attr("discord_app_server", "submit_approval_reply"),
        )(target_thread_id, answer, client=client)

    def submit_input_reply_to_app_server(
        self,
        target_thread_id: str,
        answer: str,
        *,
        client: discord_app_server.AppServerClient | None = None,
    ) -> tuple[int, str] | None:
        return cast(
            discord_app_server_bot_bridge.SubmitReplyFunc,
            self._module_attr("discord_app_server", "submit_input_reply"),
        )(target_thread_id, answer, client=client)

    def run_resident_app_server_steering_prompt(
        self,
        prompt: str,
        target_thread_id: str | None,
    ) -> discord_steering.SteeringPromptResult:
        return cast(
            Callable[[str, str | None], discord_steering.SteeringPromptResult],
            self._module_func("run_resident_app_server_steering_prompt"),
        )(prompt, target_thread_id)

    def run_ask(
        self,
        prompt: str,
        *,
        force_while_busy: bool,
        wait: bool,
        target_thread_id: str | None,
        timeout_sec: float,
    ) -> tuple[int, str]:
        return cast(discord_steering.RunAskFunc, self._module_func("run_ask"))(
            prompt,
            force_while_busy=force_while_busy,
            wait=wait,
            target_thread_id=target_thread_id,
            timeout_sec=timeout_sec,
        )

    def get_steering_bridge_module(self) -> discord_steering.SteeringBridgeLike:
        return cast(
            discord_prompt_bridge_runtime.GetSteeringBridgeModuleFunc,
            self._module_func("get_steering_bridge_module"),
        )()

    def log_line(self, message: str) -> None:
        cast(Callable[[str], None], self._module_func("log_line"))(message)

    def format_log_text_len(self, text: str) -> int | str:
        return cast(Callable[[str], int | str], self._module_func("format_log_text_len"))(text)

    def _module_func(self, name: str) -> ModuleValue:
        return cast(object, getattr(self.module, name))

    def _module_attr(self, module_name: str, attr_name: str) -> ModuleValue:
        imported_module = cast(ModuleType, getattr(self.module, module_name))
        return cast(object, getattr(imported_module, attr_name))
