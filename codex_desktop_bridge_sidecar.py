from __future__ import annotations

from collections.abc import Callable
from types import TracebackType

import codex_desktop_bridge_sidecar_io as sidecar_io
import codex_desktop_bridge_sidecar_methods as sidecar_methods
import codex_desktop_bridge_sidecar_resolver as sidecar_resolver
from codex_desktop_bridge_sidecar_lifecycle import (
    close_sidecar_process,
    ensure_stdio_available,
    start_app_server_process,
)
from codex_desktop_bridge_sidecar_process import SidecarProcess, StartProcess, start_sidecar_process
from codex_desktop_bridge_sidecar_protocol import (
    CodexSidecarError,
    CodexSidecarProcessExitedError,
)
from codex_desktop_bridge_sidecar_types import JsonObject, JsonScalar, JsonValue

CODEX_APP_SERVER_EXE = sidecar_resolver.CODEX_APP_SERVER_EXE
CODEX_APP_SERVER_EXE_ENV = sidecar_resolver.CODEX_APP_SERVER_EXE_ENV
CODEX_HOME = sidecar_resolver.CODEX_HOME
detect_running_codex_app_server_executable = sidecar_resolver.detect_running_codex_app_server_executable
is_windowsapps_path = sidecar_resolver.is_windowsapps_path
iter_codex_app_server_bin_candidates = sidecar_resolver.iter_codex_app_server_bin_candidates
normalize_executable_candidate = sidecar_resolver.normalize_executable_candidate
resolve_codex_app_server_executable = sidecar_resolver.resolve_codex_app_server_executable
run_powershell_capture = sidecar_resolver.run_powershell_capture

__all__ = [
    "CODEX_APP_SERVER_EXE",
    "CODEX_APP_SERVER_EXE_ENV",
    "CODEX_HOME",
    "CodexAppServerSidecar",
    "CodexSidecarError",
    "JsonObject",
    "JsonScalar",
    "JsonValue",
    "SidecarProcess",
    "StartProcess",
    "detect_running_codex_app_server_executable",
    "is_windowsapps_path",
    "iter_codex_app_server_bin_candidates",
    "normalize_executable_candidate",
    "resolve_codex_app_server_executable",
    "run_powershell_capture",
    "start_sidecar_process",
]


class CodexAppServerSidecar:
    def __init__(
        self,
        *,
        executable_resolver: Callable[[], str] = resolve_codex_app_server_executable,
        start_process: StartProcess = start_sidecar_process,
        initialize: bool = True,
    ) -> None:
        self.process: SidecarProcess = start_app_server_process(
            executable_resolver,
            start_process,
            app_server_exe_env=CODEX_APP_SERVER_EXE_ENV,
        )
        ensure_stdio_available(self.process)
        self._next_request_id: int = 1
        self._stdout_queue: sidecar_io.ResponseQueue = sidecar_io.new_response_queue()
        self._last_close_errors: tuple[OSError, ...] = ()
        self._start_stdout_drain_thread()
        if initialize:
            try:
                self._initialize_app_server()
            except (OSError, RuntimeError):
                self.close()
                raise

    def _start_stdout_drain_thread(self) -> None:
        self._stdout_thread: sidecar_io.StdoutThread = sidecar_io.start_stdout_drain_thread(
            self.process,
            self._stdout_queue,
        )

    def _initialize_app_server(self) -> None:
        _ = self.request(
            "initialize",
            {
                "clientInfo": {
                    "name": "codex-desktop-bridge",
                    "version": "1.0",
                },
                "capabilities": {"experimentalApi": True},
            },
            timeout_sec=5.0,
        )

    def __enter__(self) -> "CodexAppServerSidecar":
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        self.close()

    def close(self) -> None:
        self._last_close_errors = close_sidecar_process(self.process)

    def request(self, method: str, params: JsonObject, *, timeout_sec: float = 10.0) -> JsonObject:
        if self.process.poll() is not None:
            raise CodexSidecarProcessExitedError(
                f"Codex app-server sidecar exited with code {self.process.returncode}."
            )

        request_id = self._write_request(method, params)
        return self._read_response(request_id, method, timeout_sec)

    def _write_request(self, method: str, params: JsonObject) -> str:
        request_id = str(self._next_request_id)
        self._next_request_id += 1
        sidecar_io.write_sidecar_request(self.process, request_id, method, params)
        return request_id

    def _read_response(self, request_id: str, method: str, timeout_sec: float) -> JsonObject:
        return sidecar_io.read_sidecar_response(
            self._stdout_queue,
            request_id,
            method,
            timeout_sec,
        )

    def start_thread(self, cwd: str | None) -> JsonObject:
        return sidecar_methods.start_thread(self, cwd)

    def read_thread(self, thread_id: str, *, include_turns: bool = False) -> JsonObject:
        return sidecar_methods.read_thread(self, thread_id, include_turns=include_turns)

    def resume_thread(self, thread_id: str) -> JsonObject:
        return sidecar_methods.resume_thread(self, thread_id)

    def update_thread_settings(self, thread_id: str, settings: dict[str, str | None]) -> JsonObject:
        return sidecar_methods.update_thread_settings(self, thread_id, settings)

    def list_models(self) -> JsonObject:
        return sidecar_methods.list_models(self)

    def start_turn(self, thread_id: str, prompt: str) -> JsonObject:
        return sidecar_methods.start_turn(self, thread_id, prompt)

    def interrupt_turn(self, thread_id: str, turn_id: str) -> JsonObject:
        return sidecar_methods.interrupt_turn(self, thread_id, turn_id)

    def clean_background_terminals(self, thread_id: str) -> JsonObject:
        return sidecar_methods.clean_background_terminals(self, thread_id)

    def archive_thread(self, thread_id: str) -> JsonObject:
        return sidecar_methods.archive_thread(self, thread_id)
