from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
from collections.abc import Iterator
from pathlib import Path

import codex_desktop_bridge_desktop_process as desktop_process

try:
    import winreg
except ImportError:
    winreg = None


user32: ctypes.CDLL | None = ctypes.WinDLL("user32", use_last_error=True) if os.name == "nt" else None


def normalize_executable_candidate(raw: str) -> Path | None:
    cleaned = str(raw or "").strip().strip('"').strip("'")
    if not cleaned:
        return None
    if cleaned.lower().endswith(".exe,0"):
        cleaned = cleaned[:-2]
    if "," in cleaned and cleaned.lower().endswith(".exe"):
        cleaned = cleaned.split(",", 1)[0].strip()
    path = Path(cleaned).expanduser()
    if sys.platform == "darwin" and path.suffix.lower() == ".app" and path.exists() and path.is_dir():
        executable = resolve_macos_app_bundle_executable(path)
        if executable is not None:
            return executable
    if path.exists() and path.is_file():
        return path
    return None


def resolve_macos_app_bundle_executable(app_bundle: Path) -> Path | None:
    macos_dir = app_bundle / "Contents" / "MacOS"
    preferred = macos_dir / app_bundle.stem
    if preferred.exists() and preferred.is_file():
        return preferred
    if not macos_dir.exists() or not macos_dir.is_dir():
        return None
    for candidate in sorted(macos_dir.iterdir()):
        if candidate.exists() and candidate.is_file():
            return candidate
    return None


def read_registry_string(root: int, subkey: str, value_name: str = "") -> str:
    if winreg is None:
        return ""
    try:
        with winreg.OpenKey(root, subkey) as handle:
            query_result: tuple[str | bytes | int | list[str] | None, int] = winreg.QueryValueEx(handle, value_name)
    except OSError:
        return ""
    return str(query_result[0] or "").strip()


def iter_codex_desktop_registry_candidates() -> Iterator[tuple[str, Path]]:
    if winreg is None:
        return

    app_paths = (
        (winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\App Paths\ChatGPT.exe"),
        (winreg.HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\App Paths\ChatGPT.exe"),
        (winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\App Paths\Codex.exe"),
        (winreg.HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\App Paths\Codex.exe"),
    )
    for root, subkey in app_paths:
        candidate = normalize_executable_candidate(read_registry_string(root, subkey))
        if candidate is not None:
            yield (f"registry:{subkey}", candidate)

    uninstall_roots = (
        (winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (winreg.HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (winreg.HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
    )
    for root, base_subkey in uninstall_roots:
        try:
            with winreg.OpenKey(root, base_subkey) as base_handle:
                subkey_count, _, _ = winreg.QueryInfoKey(base_handle)
                subkeys = [winreg.EnumKey(base_handle, index) for index in range(subkey_count)]
        except OSError:
            continue
        for child_name in subkeys:
            child_subkey = f"{base_subkey}\\{child_name}"
            display_name = read_registry_string(root, child_subkey, "DisplayName")
            if "codex" not in display_name.lower() and "chatgpt" not in display_name.lower():
                continue
            display_icon = normalize_executable_candidate(read_registry_string(root, child_subkey, "DisplayIcon"))
            if display_icon is not None:
                yield (f"registry:{child_subkey}:DisplayIcon", display_icon)
            install_location = read_registry_string(root, child_subkey, "InstallLocation")
            for executable_name in ("ChatGPT.exe", "Codex.exe"):
                install_path = (
                    normalize_executable_candidate(str(Path(install_location) / executable_name))
                    if install_location
                    else None
                )
                if install_path is not None:
                    yield (f"registry:{child_subkey}:InstallLocation", install_path)


def iter_default_codex_desktop_candidates() -> Iterator[tuple[str, Path]]:
    if sys.platform == "darwin":
        yield from iter_macos_default_codex_desktop_candidates()
        return
    yield from desktop_process.iter_default_codex_desktop_candidates(os.environ)


def iter_macos_default_codex_desktop_candidates() -> Iterator[tuple[str, Path]]:
    for app_bundle in (
        Path("/Applications/Codex.app"),
        Path.home() / "Applications" / "Codex.app",
    ):
        executable = normalize_executable_candidate(str(app_bundle))
        if executable is not None:
            yield (f"default:{app_bundle}", executable)


def get_window_process_id(hwnd: int) -> int | None:
    if sys.platform == "darwin":
        import codex_desktop_bridge_macos_input as macos_input

        return macos_input.get_window_process_id(hwnd)
    if user32 is None:
        return None
    pid = wt.DWORD(0)
    user32.GetWindowThreadProcessId(wt.HWND(hwnd), ctypes.byref(pid))
    if not pid.value:
        return None
    return int(pid.value)


def query_process_image_path(pid: int) -> Path | None:
    if sys.platform == "darwin":
        return normalize_executable_candidate(run_macos_capture(["ps", "-p", str(int(pid)), "-o", "comm="]))

    escaped_pid = str(int(pid))
    return normalize_executable_candidate(
        run_powershell_capture(f"(Get-Process -Id {escaped_pid} -ErrorAction SilentlyContinue).Path")
    )


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


def detect_codex_desktop_executable_via_powershell() -> tuple[Path | None, str]:
    if sys.platform == "darwin":
        return detect_codex_desktop_executable_via_macos()

    if os.name != "nt":
        return (None, "")

    process_path = normalize_executable_candidate(
        run_powershell_capture(
            " ".join(
                (
                    "@('ChatGPT', 'Codex') | ForEach-Object {",
                    "Get-Process -Name $_ -ErrorAction SilentlyContinue }",
                    "| Where-Object { $_.Path }",
                    "| Select-Object -First 1 -ExpandProperty Path",
                )
            )
        )
    )
    if process_path is not None and desktop_process.is_codex_desktop_executable_candidate(process_path):
        return (process_path, "powershell:Get-Process")

    install_root = run_powershell_capture(
        " ".join(
            (
                "Get-AppxPackage OpenAI.Codex -ErrorAction SilentlyContinue",
                "| Select-Object -First 1 -ExpandProperty InstallLocation",
            )
        )
    )
    if install_root:
        for executable_name in ("ChatGPT.exe", "Codex.exe"):
            candidate = normalize_executable_candidate(str(Path(install_root) / "app" / executable_name))
            if candidate is not None:
                return (candidate, "powershell:Get-AppxPackage")

    return (None, "")


def detect_codex_desktop_executable_via_macos() -> tuple[Path | None, str]:
    for line in run_macos_capture(["ps", "-axo", "comm="]).splitlines():
        candidate = normalize_executable_candidate(line)
        if candidate is not None and "Codex.app" in str(candidate):
            return (candidate, "ps:Codex.app")

    spotlight = run_macos_capture(["mdfind", "kMDItemFSName == 'Codex.app'"])
    for line in spotlight.splitlines():
        candidate = normalize_executable_candidate(line)
        if candidate is not None:
            return (candidate, "mdfind:Codex.app")

    for source, candidate in iter_macos_default_codex_desktop_candidates():
        return (candidate, source)

    return (None, "")


def persist_env_value(path: Path, key: str, value: str) -> bool:
    path = Path(path)
    newline = "\n"
    lines: list[str] = []
    if path.exists():
        text = path.read_text(encoding="utf-8")
        newline = "\r\n" if "\r\n" in text else "\n"
        lines = text.splitlines()
    found = False
    changed = False
    for index, raw_line in enumerate(lines):
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#") or "=" not in raw_line:
            continue
        existing_key, _, _ = raw_line.partition("=")
        if existing_key.strip() != key:
            continue
        found = True
        replacement = f"{key}={value}"
        if raw_line != replacement:
            lines[index] = replacement
            changed = True
        break
    if not found:
        lines.append(f"{key}={value}")
        changed = True
    if not changed:
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    return path.write_text(newline.join(lines) + newline, encoding="utf-8") > 0
