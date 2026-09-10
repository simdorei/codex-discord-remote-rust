from __future__ import annotations

import threading
from pathlib import Path
from collections.abc import Callable
from typing import IO, Final, Protocol, final


APP_SERVER_STDERR_LOG_NAME: Final = "codex_app_server_stderr.log"
APP_SERVER_STDERR_MAX_BYTES: Final = 5 * 1024 * 1024
APP_SERVER_STDERR_FILE_COUNT: Final = 3
_ROTATION_LOCK = threading.Lock()
LogFunc = Callable[[str], None]


class StderrProcess(Protocol):
    @property
    def stderr(self) -> IO[str] | None: ...


@final
class RotatingAppServerStderrLog:
    """Bounded UTF-8 sink for raw app-server stderr diagnostics."""

    def __init__(
        self,
        path: Path,
        *,
        max_bytes: int = APP_SERVER_STDERR_MAX_BYTES,
        file_count: int = APP_SERVER_STDERR_FILE_COUNT,
    ) -> None:
        if max_bytes <= 0:
            raise ValueError("max_bytes must be positive")
        if file_count <= 0:
            raise ValueError("file_count must be positive")
        self._path = path
        self._max_bytes = max_bytes
        self._file_count = file_count

    def write(self, text: str) -> None:
        with _ROTATION_LOCK:
            self._path.parent.mkdir(parents=True, exist_ok=True)
            offset = 0
            while offset < len(text):
                current_size = self._path.stat().st_size if self._path.exists() else 0
                if current_size >= self._max_bytes:
                    self._rotate_locked()
                    current_size = 0
                encoded, offset = self._encode_chunk(
                    text,
                    offset=offset,
                    capacity=self._max_bytes - current_size,
                )
                with self._path.open("ab") as stream:
                    _ = stream.write(encoded)
                    stream.flush()

    def close(self) -> None:
        return

    def _rotate_locked(self) -> None:
        oldest_index = self._file_count - 1
        if oldest_index > 0:
            self._backup_path(oldest_index).unlink(missing_ok=True)
        for index in range(oldest_index - 1, 0, -1):
            source = self._backup_path(index)
            if source.exists():
                source.replace(self._backup_path(index + 1))
        if self._file_count > 1 and self._path.exists():
            self._path.replace(self._backup_path(1))
        elif self._path.exists():
            self._path.unlink()

    def _backup_path(self, index: int) -> Path:
        return self._path.with_name(f"{self._path.name}.{index}")

    def _encode_chunk(
        self,
        text: str,
        *,
        offset: int,
        capacity: int,
    ) -> tuple[bytes, int]:
        chunk = bytearray()
        while offset < len(text):
            encoded_char = text[offset].encode("utf-8", errors="replace")
            if len(encoded_char) > capacity and not chunk:
                if capacity < self._max_bytes:
                    self._rotate_locked()
                    capacity = self._max_bytes
                    continue
                encoded_char = b"?"
            if len(chunk) + len(encoded_char) > capacity:
                break
            chunk.extend(encoded_char)
            offset += 1
        return bytes(chunk), offset


@final
class AppServerStderrRecorder:
    def __init__(
        self,
        process: StderrProcess,
        path: Path,
        *,
        log: LogFunc,
    ) -> None:
        self._process = process
        self._sink = RotatingAppServerStderrLog(path)
        self._log = log
        self._thread = threading.Thread(target=self._drain, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def close(self) -> None:
        self._thread.join(timeout=2.0)
        if self._thread.is_alive():
            self._log("app_server_stderr_recorder_close_timed_out")
        self._sink.close()

    def _drain(self) -> None:
        stderr = self._process.stderr
        if stderr is None:
            return
        try:
            for line in stderr:
                try:
                    self._sink.write(line)
                except OSError as exc:
                    self._log(
                        "app_server_stderr_log_write_failed "
                        + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
                    )
        except OSError as exc:
            self._log(
                "app_server_stderr_pipe_read_failed "
                + f"error_type={type(exc).__name__} error={str(exc)[:300]}"
            )
