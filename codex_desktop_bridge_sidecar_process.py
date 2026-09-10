from __future__ import annotations

import subprocess
from collections.abc import Iterable
from typing import IO, Protocol


class TextInput(Protocol):
    @property
    def closed(self) -> bool: ...

    def write(self, value: str, /) -> int: ...
    def flush(self) -> None: ...
    def close(self) -> None: ...


class SidecarProcess(Protocol):
    @property
    def stdin(self) -> TextInput | None: ...

    @property
    def stdout(self) -> Iterable[str] | None: ...

    @property
    def returncode(self) -> int | None: ...

    def poll(self) -> int | None: ...
    def terminate(self) -> None: ...
    def wait(self, timeout: float | None = None) -> int: ...
    def kill(self) -> None: ...


class StartProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        stdin: int,
        stdout: int,
        stderr: int,
        text: bool,
        encoding: str,
        bufsize: int,
        creationflags: int,
    ) -> SidecarProcess: ...


class _PopenSidecarProcess:
    def __init__(self, process: subprocess.Popen[str]) -> None:
        self._process: subprocess.Popen[str] = process

    @property
    def stdin(self) -> TextInput | None:
        stream: IO[str] | None = self._process.stdin
        return stream

    @property
    def stdout(self) -> Iterable[str] | None:
        stream: IO[str] | None = self._process.stdout
        return stream

    @property
    def returncode(self) -> int | None:
        return self._process.returncode

    def poll(self) -> int | None:
        return self._process.poll()

    def terminate(self) -> None:
        self._process.terminate()

    def wait(self, timeout: float | None = None) -> int:
        return self._process.wait(timeout=timeout)

    def kill(self) -> None:
        self._process.kill()


def start_sidecar_process(
    args: list[str],
    *,
    stdin: int,
    stdout: int,
    stderr: int,
    text: bool,
    encoding: str,
    bufsize: int,
    creationflags: int,
) -> SidecarProcess:
    return _PopenSidecarProcess(
        subprocess.Popen(
            args,
            stdin=stdin,
            stdout=stdout,
            stderr=stderr,
            text=text,
            encoding=encoding,
            bufsize=bufsize,
            creationflags=creationflags,
        )
    )
