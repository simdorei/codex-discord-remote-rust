"""Process helpers for running the local Codex Desktop bridge."""

from __future__ import annotations

import argparse
import io
import os
import subprocess
import sys
from collections.abc import Callable
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from types import TracebackType
from typing import IO, Protocol, cast, runtime_checkable

LineCallback = Callable[[str], None]


class BridgeModule(Protocol):
    def build_parser(self) -> argparse.ArgumentParser: ...


class RedirectLock(Protocol):
    def __enter__(self) -> bool | None: ...
    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
        /,
    ) -> bool | None: ...


@runtime_checkable
class CommandFunc(Protocol):
    def __call__(self, args: argparse.Namespace) -> int | None: ...


class MissingBridgeCommandHandlerError(RuntimeError):
    pass


def run_bridge_command(
    argv: list[str],
    *,
    bridge_module: BridgeModule,
    stream_redirect_lock: RedirectLock,
) -> tuple[int, str]:
    parser = bridge_module.build_parser()
    stdout_buffer = io.StringIO()
    stderr_buffer = io.StringIO()
    exit_code = 0
    with stream_redirect_lock, redirect_stdout(stdout_buffer), redirect_stderr(stderr_buffer):
        try:
            args = parser.parse_args(argv)
            func = getattr(args, "func", None)
            if not isinstance(func, CommandFunc):
                raise MissingBridgeCommandHandlerError("missing bridge command handler")
            result = func(args)
            exit_code = int(result or 0)
        except SystemExit as exc:
            exit_code = exc.code if isinstance(exc.code, int) else 1
        except Exception as exc:  # noqa: BROAD_EXCEPT_OK
            exit_code = 1
            print(f"ERROR: {exc}")
    output = stdout_buffer.getvalue()
    stderr = stderr_buffer.getvalue()
    combined = (output + ("\n" + stderr if stderr else "")).strip()
    return exit_code, combined


def parse_bridge_output_value(output: str, key: str) -> str | None:
    prefix = f"{key}:"
    for line in (output or "").splitlines():
        if line.startswith(prefix):
            value = line[len(prefix) :].strip()
            return value or None
    return None


def get_bridge_script_path(script_dir: Path) -> Path:
    return script_dir / "codex_desktop_bridge.py"


def build_bridge_subprocess_env() -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONIOENCODING"] = "utf-8"
    env["PYTHONUTF8"] = "1"
    env["PYTHONUNBUFFERED"] = "1"
    return env


def run_bridge_command_stream(
    argv: list[str],
    on_line: LineCallback,
    *,
    script_path: Path,
    cwd: Path,
    env: dict[str, str],
) -> tuple[int, str]:
    output_parts: list[str] = []
    command = [sys.executable, str(script_path), *argv]
    try:
        process: subprocess.Popen[str] = subprocess.Popen(
            command,
            cwd=str(cwd),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        try:
            stdout = cast(IO[str] | None, process.stdout)
            if stdout is not None:
                for raw_line in stdout:
                    output_parts.append(raw_line)
                    on_line(raw_line.rstrip("\r\n"))
        finally:
            stdout = cast(IO[str] | None, process.stdout)
            if stdout is not None:
                stdout.close()
        exit_code = process.wait()
        return int(exit_code or 0), "".join(output_parts).strip()
    except Exception as exc:  # noqa: BROAD_EXCEPT_OK
        output = f"ERROR: {exc}"
        on_line(output)
        return 1, output
