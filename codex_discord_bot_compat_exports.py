from __future__ import annotations
import argparse
import asyncio  # noqa: ANYIO_OK
import ctypes
import importlib
import io
import json
import os
import re
import sqlite3
from collections.abc import Awaitable, Callable, Mapping
from contextlib import AbstractAsyncContextManager, asynccontextmanager, contextmanager
from dataclasses import dataclass
from types import ModuleType, SimpleNamespace
from typing import TypeAlias, cast, override

from codex_discord_bot_compat_export_tables import EXPORTS_BY_MODULE, MODULE_ALIASES
from codex_discord_bot_module_context import install_runtime_bindings

ModuleValue: TypeAlias = object
SendChunksFunc: TypeAlias = Callable[..., Awaitable[ModuleValue]]

class LineStream(io.TextIOBase):
    on_line: Callable[[str], None]
    _buffer: str

    def __init__(self, on_line: Callable[[str], None]) -> None:
        self.on_line = on_line
        self._buffer = ""
        self._all: list[str] = []

    @override
    def write(self, text: str) -> int:
        if not text:
            return 0
        self._all.append(text)
        self._buffer += text
        while "\n" in self._buffer:
            line, self._buffer = self._buffer.split("\n", 1)
            self.on_line(line.rstrip("\r"))
        return len(text)

    @override
    def flush(self) -> None:
        if self._buffer:
            self.on_line(self._buffer.rstrip("\r"))
            self._buffer = ""

    def getvalue(self) -> str:
        return "".join(self._all)


_STD_EXPORTS: Mapping[str, ModuleValue] = {
    "argparse": argparse,
    "asynccontextmanager": asynccontextmanager,
    "contextmanager": contextmanager,
    "ctypes": ctypes,
    "json": json,
    "LineStream": LineStream,
    "os": os,
    "re": re,
    "SimpleNamespace": SimpleNamespace,
}

@dataclass(frozen=True, slots=True)
class BotCompatExportsRuntime:
    module: ModuleType

    def install(self) -> None:
        for name, value in _STD_EXPORTS.items():
            self._set(name, value)
        self._set("app_commands", importlib.import_module("discord.app_commands"))
        for alias, module_name in MODULE_ALIASES.items():
            self._set(alias, importlib.import_module(module_name))
        for module_name, export_names in EXPORTS_BY_MODULE.items():
            source = importlib.import_module(module_name)
            for export_name in export_names:
                self._set(export_name, cast(ModuleValue, getattr(source, export_name)))
        self._set("DISCORD_DELIVERY_STOPPING", cast(bool, getattr(self.module, "discord_delivery_stopping")))
        self._install_wrappers()

    def _install_wrappers(self) -> None:
        self._set("build_parser", self.build_parser)
        self._set("format_pending_delivery_output", self.format_pending_delivery_output)
        self._set("get_discord_session_mirror_poll_seconds", self.get_discord_session_mirror_poll_seconds)
        self._set("get_session_mirror_archive_backlog_max_events", self.get_session_mirror_archive_backlog_max_events)
        self._set("is_delivery_confirmation_timeout", self.is_delivery_confirmation_timeout)
        self._set("is_stale_busy_thread_for_steering", self.is_stale_busy_thread_for_steering)
        self._set("mark_persistent_discord_message_processed", self.mark_persistent_discord_message_processed)
        self._set("run_app_server_prompt_no_wait", self.run_app_server_prompt_no_wait)
        self._set("run_app_server_steering_prompt", self.run_app_server_steering_prompt)
        self._set("run_delayed_prompt_via_watch", self.run_delayed_prompt_via_watch)
        self._set("run_resident_app_server_prompt_no_wait", self.run_resident_app_server_prompt_no_wait)
        self._set("submit_app_server_approval_reply", self.submit_app_server_approval_reply)
        self._set("submit_resident_app_server_approval_reply", self.submit_resident_app_server_approval_reply)

    def build_parser(self) -> argparse.ArgumentParser:
        parser = argparse.ArgumentParser(description="Discord adapter for codex_desktop_bridge.py")
        _ = parser.add_argument(
            "--no-message-content",
            action="store_true",
            help="Disable prefix/plain-message handling and use slash commands only.",
        )
        return parser

    def get_discord_session_mirror_poll_seconds(self) -> float:
        return cast(
            Callable[..., float],
            getattr(self.module, "parse_bounded_float_env"),
        )("DISCORD_SESSION_MIRROR_POLL_SECONDS", default=1.0, minimum=0.25, maximum=60.0)

    def get_session_mirror_archive_backlog_max_events(self) -> int:
        return cast(
            Callable[..., int],
            getattr(self.module, "parse_bounded_int_arg"),
        )(
            os.environ.get("DISCORD_SESSION_MIRROR_ARCHIVE_BACKLOG_MAX_EVENTS", ""),
            default=cast(int, getattr(self.module, "SESSION_MIRROR_ARCHIVE_BACKLOG_MAX_EVENTS_DEFAULT")),
            minimum=0,
            maximum=10000,
        )

    def mark_persistent_discord_message_processed(self, message_id: int, now: float | None = None) -> None:
        try:
            cast(Callable[..., None], getattr(self.module, "discord_store").mark_processed_discord_message_id)(
                getattr(self.module, "MIRROR_DB_PATH"),
                message_id,
                now=now,
            )
        except (OSError, RuntimeError, sqlite3.Error) as exc:
            self._log(f"processed_message_mark_failed message={message_id} error_type={type(exc).__name__}")

    def run_resident_app_server_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        app_server = importlib.import_module("codex_discord_app_server")
        transport = importlib.import_module("codex_app_server_transport")
        run_prompt_no_wait = cast(Callable[..., tuple[int, str]], getattr(app_server, "run_prompt_no_wait"))
        return run_prompt_no_wait(
            prompt,
            target_thread_id,
            transport_module=transport,
            bridge_module=importlib.import_module("codex_desktop_bridge"),
            client=cast(ModuleValue, getattr(transport, "DEFAULT_CLIENT")),
            confirm_timeout_sec=6.0,
        )

    def run_app_server_prompt_no_wait(self, prompt: str, target_thread_id: str | None) -> tuple[int, str]:
        return self.run_resident_app_server_prompt_no_wait(prompt, target_thread_id)

    def run_app_server_steering_prompt(self, prompt: str, target_thread_id: str | None) -> ModuleValue:
        return cast(Callable[..., ModuleValue], getattr(self.module, "run_resident_app_server_steering_prompt"))(
            prompt,
            target_thread_id,
        )

    def submit_resident_app_server_approval_reply(self, target_thread_id: str, answer: str) -> tuple[int, str] | None:
        if not cast(Callable[[], bool], getattr(self.module, "app_server_transport_enabled"))():
            return None
        transport = importlib.import_module("codex_app_server_transport")
        app_server = importlib.import_module("codex_discord_app_server")
        submit_approval_reply = cast(Callable[..., tuple[int, str]], getattr(app_server, "submit_approval_reply"))
        return submit_approval_reply(
            target_thread_id,
            answer,
            client=cast(ModuleValue, getattr(transport, "DEFAULT_CLIENT")),
        )

    def submit_app_server_approval_reply(self, target_thread_id: str, answer: str) -> tuple[int, str] | None:
        return self.submit_resident_app_server_approval_reply(target_thread_id, answer)

    def is_delivery_confirmation_timeout(self, output: str) -> bool:
        steering = cast(ModuleType, getattr(self.module, "discord_steering"))
        checker = cast(Callable[[str], bool], getattr(steering, "is_ipc_delivery_confirmation_timeout"))
        return checker(output)

    def format_pending_delivery_output(self, output: str) -> str:
        steering = cast(ModuleType, getattr(self.module, "discord_steering"))
        formatter = cast(Callable[[str], str], getattr(steering, "format_pending_ipc_delivery_output"))
        return formatter(output)

    def is_stale_busy_thread_for_steering(self, target_thread_id: str | None) -> bool:
        block_info = cast(Callable[[str | None], ModuleValue | None], getattr(self.module, "get_stale_busy_steer_block_info"))
        return block_info(target_thread_id) is not None

    async def run_delayed_prompt_via_watch(
        self,
        channel: ModuleValue,
        prompt: str,
        *,
        target_thread_id: str | None,
        delegate_to_session_mirror: bool,
    ) -> None:
        channel_typing = cast(Callable[..., AbstractAsyncContextManager[None]], getattr(self.module, "channel_typing"))
        async with channel_typing(channel, context="ask_after_cross_session_wait"):
            steering_result = cast(
                tuple[int, str],
                await asyncio.to_thread(
                    cast(Callable[..., ModuleValue], getattr(self.module, "run_steering_prompt")),
                    prompt,
                    target_thread_id,
                ),
            )
        exit_code, output = steering_result
        log_message = f"ask_after_cross_session_wait_done exit={exit_code} target={target_thread_id or '-'} "
        log_message += f"pending={getattr(steering_result, 'delivery_pending', False)} "
        log_message += f"output_len={cast(Callable[[str], str], getattr(self.module, 'format_log_text_len'))(output)}"
        self._log(log_message)
        if cast(Callable[[int, str], bool], getattr(self.module, "is_selected_thread_busy_error"))(exit_code, output):
            sent_menu = await cast(Callable[..., Awaitable[bool]], getattr(self.module, "send_codex_app_menu_if_available"))(
                channel,
                target_thread_id,
                output,
                reason="ask_after_cross_session_wait_busy",
            )
            if sent_menu:
                return
            _resolved_thread_id, target_ref = cast(
                Callable[[str | None], tuple[str | None, str]],
                getattr(self.module, "resolve_target_ref"),
            )(target_thread_id)
            _ = await cast(SendChunksFunc, getattr(self.module, "send_chunks"))(
                channel,
                cast(Callable[[str], str], getattr(self.module, "build_codex_app_steering_not_accepted_message"))(target_ref),
            )
            return
        if exit_code != 0:
            _ = await cast(SendChunksFunc, getattr(self.module, "send_chunks"))(
                channel,
                f"Ask failed (exit {exit_code})\n\n{output or '(no output)'}",
            )
            return
        cast(Callable[[str | None], None], getattr(self.module, "mark_steering_handoff"))(target_thread_id)
        streamed = await cast(Callable[..., Awaitable[bool]], getattr(self.module, "stream_steering_prompt_result_to_channel"))(
            channel,
            steering_result,
            target_thread_id,
            label="Ask",
            send_commentary_blocks=False if delegate_to_session_mirror else None,
            send_final_blocks=not delegate_to_session_mirror,
        )
        if streamed:
            return
        _ = await cast(SendChunksFunc, getattr(self.module, "send_chunks"))(
            channel,
            f"Ask sent\n\n{output or '(no output)'}",
        )

    def _log(self, message: str) -> None:
        cast(Callable[[str], None], getattr(self.module, "log_line"))(message)

    def _set(self, name: str, value: ModuleValue) -> None:
        install_runtime_bindings(
            self.module,
            owner=type(self).__name__,
            values={name: value},
        )
