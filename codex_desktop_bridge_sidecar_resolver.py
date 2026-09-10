from __future__ import annotations

import os
import shutil
import subprocess
import sys
from collections.abc import Callable, Iterator
from pathlib import Path

CODEX_HOME = Path(os.environ.get("CODEX_HOME", "")).expanduser() if os.environ.get("CODEX_HOME") else Path.home() / ".codex"
CODEX_APP_SERVER_EXE_ENV = "CODEX_EXE"
CODEX_APP_SERVER_EXE = os.environ.get(CODEX_APP_SERVER_EXE_ENV, "").strip()
LogFunc = Callable[[str], None]


def clean_executable_candidate(raw: str) -> str:
    cleaned = str(raw or "").strip().strip('"').strip("'")
    if not cleaned:
        return ""
    if cleaned.lower().endswith(".exe,0"):
        cleaned = cleaned[:-2]
    if "," in cleaned and cleaned.lower().endswith(".exe"):
        cleaned = cleaned.split(",", 1)[0].strip()
    return os.path.expandvars(os.path.expanduser(cleaned))


def normalize_executable_candidate(raw: str) -> Path | None:
    cleaned = clean_executable_candidate(raw)
    if not cleaned:
        return None
    path = Path(cleaned)
    if path.exists() and path.is_file():
        return path
    return None


def resolve_configured_executable(raw: str) -> Path | None:
    cleaned = clean_executable_candidate(raw)
    candidate = normalize_executable_candidate(cleaned)
    if candidate is not None:
        return candidate
    resolved = shutil.which(cleaned) if cleaned else None
    return normalize_executable_candidate(resolved or "")


def selected_executable(
    executable: Path | str,
    *,
    source: str,
    log_func: LogFunc | None,
) -> str:
    selected = str(executable)
    if log_func is not None:
        log_func(f"codex_executable_selected source={source} executable={selected}")
    return selected


def is_windowsapps_path(path: Path | str) -> bool:
    return "\\windowsapps\\" in str(path).lower()


def run_powershell_capture(command: str) -> str:
    if os.name != "nt":
        return ""

    powershell_exe = (
        shutil.which("powershell.exe")
        or shutil.which("powershell")
        or r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
    )
    completed = subprocess.run(
        [powershell_exe, "-NoProfile", "-Command", command],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
    )
    if completed.returncode != 0:
        return ""
    return completed.stdout.strip()


def run_macos_capture(args: list[str]) -> str:
    if sys.platform != "darwin":
        return ""
    completed = subprocess.run(
        args,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if completed.returncode != 0:
        return ""
    return completed.stdout.strip()


def detect_running_codex_app_server_executable() -> tuple[Path | None, str]:
    if sys.platform == "darwin":
        for line in run_macos_capture(["ps", "-axo", "comm=,args="]).splitlines():
            if " app-server" not in line:
                continue
            command = line.split(None, 1)[0]
            candidate = normalize_executable_candidate(command)
            if candidate is not None:
                return (candidate, "ps:running-app-server")
        return (None, "")

    process_path = normalize_executable_candidate(
        run_powershell_capture(
            "Get-CimInstance Win32_Process " +
            "| Where-Object { $_.Name -ieq 'codex.exe' -and $_.CommandLine -match 'app-server' -and $_.ExecutablePath } " +
            "| Sort-Object @{ Expression = { if ($_.ExecutablePath -like '*\\\\WindowsApps\\\\*') { 1 } else { 0 } } } " +
            "| Select-Object -First 1 -ExpandProperty ExecutablePath"
        )
    )
    if process_path is not None and not is_windowsapps_path(process_path):
        return (process_path, "powershell:running-app-server")
    return (None, "")


def iter_codex_app_server_bin_candidates() -> Iterator[tuple[str, Path]]:
    roots: list[Path] = []
    local_app_data = os.environ.get("LOCALAPPDATA", "").strip()
    if local_app_data:
        roots.append(Path(local_app_data) / "OpenAI" / "Codex" / "bin")
    if os.name == "nt":
        fallback_root = Path.home() / "AppData" / "Local" / "OpenAI" / "Codex" / "bin"
        if all(str(fallback_root).lower() != str(root).lower() for root in roots):
            roots.append(fallback_root)
    if sys.platform == "darwin":
        roots.extend(
            [
                Path.home() / "Library" / "Application Support" / "OpenAI" / "Codex" / "bin",
                Path("/Applications/Codex.app/Contents/Resources/bin"),
            ]
        )

    seen: set[str] = set()
    candidates: list[Path] = []
    executable_name = "codex.exe" if os.name == "nt" else "codex"
    for root in roots:
        if not root.exists():
            continue
        root_candidate = root / executable_name
        if root_candidate.exists() and root_candidate.is_file():
            candidates.append(root_candidate)
            seen.add(str(root_candidate).lower())
        for candidate in root.glob(f"*/{executable_name}"):
            normalized = str(candidate).lower()
            if normalized in seen:
                continue
            seen.add(normalized)
            if candidate.exists() and candidate.is_file():
                candidates.append(candidate)

    candidates.sort(key=lambda path: path.stat().st_mtime, reverse=True)
    for candidate in candidates:
        yield (f"local-app-bin:{candidate.parent}", candidate)


def resolve_codex_app_server_executable(*, log_func: LogFunc | None = None) -> str:
    if CODEX_APP_SERVER_EXE:
        configured = resolve_configured_executable(CODEX_APP_SERVER_EXE)
        if configured is not None:
            return selected_executable(configured, source="env", log_func=log_func)
        if log_func is not None:
            log_func(
                "codex_executable_config_skipped "
                + f"source=env configured={CODEX_APP_SERVER_EXE!r} "
                + "reason=not_found_or_not_executable"
            )

    bundled_name = "codex.exe" if os.name == "nt" else "codex"
    bundled_path = CODEX_HOME / ".sandbox-bin" / bundled_name
    if bundled_path.exists():
        return selected_executable(bundled_path, source="sandbox-bin", log_func=log_func)

    running_path, _running_source = detect_running_codex_app_server_executable()
    if running_path is not None:
        return selected_executable(running_path, source="running-process", log_func=log_func)

    for _source, candidate in iter_codex_app_server_bin_candidates():
        return selected_executable(candidate, source="local-app-bin", log_func=log_func)

    windowsapps_candidate = ""
    for candidate in ("codex", "codex.exe"):
        resolved = shutil.which(candidate)
        if resolved:
            if is_windowsapps_path(resolved):
                windowsapps_candidate = resolved
                continue
            return selected_executable(resolved, source="PATH", log_func=log_func)

    if windowsapps_candidate:
        raise RuntimeError(
            "Codex app-server executable resolved only to a WindowsApps alias. " +
            f"Set {CODEX_APP_SERVER_EXE_ENV} in .env to the real Codex CLI executable."
        )

    return selected_executable(bundled_name, source="default-name", log_func=log_func)
