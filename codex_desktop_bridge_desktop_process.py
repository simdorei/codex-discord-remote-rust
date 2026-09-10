from __future__ import annotations

import os
import subprocess
import sys
from collections.abc import Callable, Iterable, Iterator, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Protocol

from codex_desktop_bridge_desktop_process_scripts import build_stop_codex_app_server_script

GetOptionalEnvFilePath = Callable[[str], Path | None]
DiscoverExecutable = Callable[[], tuple[Path | None, str]]
IterCandidates = Callable[[], Iterable[tuple[str, Path]]]
PersistEnvValue = Callable[[Path, str, str], bool]
SetEnvironValue = Callable[[str, str], None]
WhichExecutable = Callable[[str], str | None]
CHATGPT_APP_USER_MODEL_ID: Final = "OpenAI.Codex_2p2nqsd0c76g0!App"


class RunProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        capture_output: bool,
        text: bool,
        encoding: str,
        errors: str,
        creationflags: int,
    ) -> subprocess.CompletedProcess[str]: ...


class StartProcess(Protocol):
    def __call__(
        self,
        args: list[str],
        *,
        cwd: str,
        close_fds: bool,
        creationflags: int,
        text: bool,
    ) -> StartedDesktopProcess: ...


class StartedDesktopProcess(Protocol):
    @property
    def pid(self) -> int: ...

    def poll(self) -> int | None: ...


@dataclass(frozen=True, slots=True)
class ShellActivatedDesktopProcess:
    launcher: StartedDesktopProcess

    @property
    def pid(self) -> int:
        return self.launcher.pid

    def poll(self) -> None:
        return None


@dataclass(frozen=True, slots=True)
class DesktopProcessDeps:
    get_optional_env_file_path: GetOptionalEnvFilePath
    detect_running_codex_desktop_executable: DiscoverExecutable
    detect_codex_desktop_executable_via_powershell: DiscoverExecutable
    iter_codex_desktop_registry_candidates: IterCandidates
    iter_default_codex_desktop_candidates: IterCandidates
    persist_env_value: PersistEnvValue
    set_environ_value: SetEnvironValue
    which: WhichExecutable
    run_process: RunProcess
    start_process: StartProcess
    create_no_window: int = 0
    create_new_process_group: int = 0


def iter_default_codex_desktop_candidates(environ: Mapping[str, str]) -> Iterator[tuple[str, Path]]:
    raw_candidates = [
        environ.get("LOCALAPPDATA", "").strip(),
        environ.get("ProgramFiles", "").strip(),
        environ.get("ProgramFiles(x86)", "").strip(),
    ]
    seen: set[str] = set()
    for base in raw_candidates:
        if not base:
            continue
        for candidate in (
            Path(base) / "Programs" / "ChatGPT" / "ChatGPT.exe",
            Path(base) / "ChatGPT" / "ChatGPT.exe",
            Path(base) / "Programs" / "Codex" / "ChatGPT.exe",
            Path(base) / "Codex" / "ChatGPT.exe",
            Path(base) / "Programs" / "Codex" / "Codex.exe",
            Path(base) / "Codex" / "Codex.exe",
        ):
            normalized = str(candidate).lower()
            if normalized in seen:
                continue
            seen.add(normalized)
            if candidate.exists() and candidate.is_file():
                yield (f"default:{candidate.parent}", candidate)


def is_codex_desktop_executable_candidate(path: Path) -> bool:
    if path.name.lower() != "codex.exe":
        return True
    if path.parent.name.lower() != "resources":
        return True
    return path.parent.parent.name.lower() != "app"


def discover_codex_desktop_executable(
    *,
    env_name: str,
    deps: DesktopProcessDeps,
) -> tuple[Path | None, str]:
    env_path = deps.get_optional_env_file_path(env_name)
    if env_path is not None and is_codex_desktop_executable_candidate(env_path):
        return (env_path, f"env:{env_name}")

    running_path, running_source = deps.detect_running_codex_desktop_executable()
    if running_path is not None and is_codex_desktop_executable_candidate(running_path):
        return (running_path, running_source)

    powershell_path, powershell_source = deps.detect_codex_desktop_executable_via_powershell()
    if powershell_path is not None and is_codex_desktop_executable_candidate(powershell_path):
        return (powershell_path, powershell_source)

    for source, candidate in deps.iter_codex_desktop_registry_candidates():
        if is_codex_desktop_executable_candidate(candidate):
            return (candidate, source)

    for source, candidate in deps.iter_default_codex_desktop_candidates():
        if is_codex_desktop_executable_candidate(candidate):
            return (candidate, source)

    return (None, "")


def ensure_codex_desktop_executable_configured(
    *,
    bridge_env_path: Path,
    env_name: str,
    deps: DesktopProcessDeps,
) -> tuple[Path, str, bool]:
    discovered_path, source = discover_codex_desktop_executable(env_name=env_name, deps=deps)
    if discovered_path is None:
        raise RuntimeError(
            "Codex Desktop executable could not be discovered. "
            + "Set CODEX_DESKTOP_EXE in .env or install Codex Desktop in the default platform location."
        )
    updated = deps.persist_env_value(bridge_env_path, env_name, str(discovered_path))
    deps.set_environ_value(env_name, str(discovered_path))
    return (discovered_path, source or "discovered", updated)


def stop_codex_desktop_processes(
    executable_path: Path,
    *,
    deps: DesktopProcessDeps,
) -> tuple[bool, str]:
    if sys.platform == "darwin":
        app_name = _macos_app_name_for_executable(executable_path)
        completed = deps.run_process(
            ["osascript", "-e", f'tell application "{app_name}" to quit'],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            creationflags=0,
        )
        details = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
        return (completed.returncode == 0, details or "-")

    taskkill = deps.which("taskkill") or "taskkill"
    completed = deps.run_process(
        [taskkill, "/IM", executable_path.name, "/F"],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        creationflags=deps.create_no_window,
    )
    details = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
    stopped = completed.returncode == 0
    return (stopped, details or "-")


def stop_codex_app_server_processes() -> tuple[bool, str]:
    if sys.platform == "darwin":
        completed = subprocess.run(
            ["pkill", "-f", "codex.*app-server"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
        )
        details = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
        if completed.returncode == 1:
            return (False, details or "no matching app-server processes")
        return (completed.returncode == 0, details or "-")

    if os.name != "nt":
        return (False, "skipped: app-server process stop is only implemented on Windows")

    script = build_stop_codex_app_server_script()
    completed = subprocess.run(
        ["powershell", "-NoProfile", "-Command", script],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    details = "\n".join(part for part in [completed.stdout.strip(), completed.stderr.strip()] if part).strip()
    stopped = "stopped PID=" in details
    return (stopped, details or "-")


def start_codex_desktop_process(
    executable_path: Path,
    *,
    deps: DesktopProcessDeps,
) -> StartedDesktopProcess:
    creationflags = 0
    creationflags |= deps.create_new_process_group
    if sys.platform == "darwin":
        app_bundle = _macos_app_bundle_for_executable(executable_path)
        if app_bundle is not None:
            return deps.start_process(
                ["open", str(app_bundle)],
                cwd=str(app_bundle.parent),
                close_fds=True,
                creationflags=0,
                text=True,
            )
    if (
        sys.platform == "win32"
        and executable_path.name.casefold() == "chatgpt.exe"
        and any(parent.name.casefold() == "windowsapps" for parent in executable_path.parents)
    ):
        explorer = deps.which("explorer") or "explorer.exe"
        launcher = deps.start_process(
            [explorer, f"shell:AppsFolder\\{CHATGPT_APP_USER_MODEL_ID}"],
            cwd=str(executable_path.parent),
            close_fds=True,
            creationflags=0,
            text=True,
        )
        return ShellActivatedDesktopProcess(launcher)
    return deps.start_process(
        [str(executable_path)],
        cwd=str(executable_path.parent),
        close_fds=True,
        creationflags=creationflags,
        text=True,
    )


def _macos_app_bundle_for_executable(executable_path: Path) -> Path | None:
    for parent in [executable_path, *executable_path.parents]:
        if parent.suffix.lower() == ".app":
            return parent
    return None


def _macos_app_name_for_executable(executable_path: Path) -> str:
    app_bundle = _macos_app_bundle_for_executable(executable_path)
    if app_bundle is not None:
        return app_bundle.stem
    return executable_path.stem
