from __future__ import annotations

import json
import subprocess
import sys
import textwrap
import time
from pathlib import Path
from typing import Literal

from tests.install_plugin_contract_support import (
    FailureCommand,
    prepare_installer_repo,
)
from tests.windows_fake_codex import compile_fake_codex_exe

SuccessStderrCommand = Literal["marketplace_list", "plugin_list"]
CodexLauncher = Literal["cmd", "exe", "ps1"]


def _unlink_executable_with_retry(path: Path) -> None:
    for attempt in range(40):
        try:
            path.unlink(missing_ok=True)
            return
        except PermissionError:
            if attempt == 39:
                raise
            time.sleep(0.05)


def run_windows_installer(
    repo_root: Path,
    marketplace_json: str,
    plugin_json: str,
    *,
    failure_command: FailureCommand | None = None,
    success_stderr_command: SuccessStderrCommand | None = None,
    utf8_inventory_output: bool = False,
    codex_launcher: CodexLauncher = "cmd",
    large_failure_output: bool = False,
    use_system_python: bool = False,
    dry_run: bool = False,
) -> subprocess.CompletedProcess[str]:
    installer = prepare_installer_repo(repo_root, "install.ps1")
    _ = (repo_root / "codex_desktop_bridge.py").write_text(
        "raise SystemExit(0)\n",
        encoding="utf-8",
    )
    fake_python = repo_root / "fake-python.cmd"
    _ = fake_python.write_text(
        textwrap.dedent(
            f"""\
            @echo off
            if /I not "%~nx1"=="verify_codex_plugin_inventory.py" exit /b 0
            "{sys.executable}" %*
            """
        ).replace("\n", "\r\n"),
        encoding="utf-8",
    )
    failure_lines = {
        "marketplace_add": 'if "%~1|%~2|%~3"=="plugin|marketplace|add" (echo codex stdout diagnostic & echo error: unrecognized subcommand marketplace 1>&2 & exit /b 7)',
        "plugin_add": 'if "%~1|%~2"=="plugin|add" (echo codex stdout diagnostic & echo error: unrecognized subcommand add 1>&2 & exit /b 7)',
        "marketplace_list": 'if "%~1|%~2|%~3"=="plugin|marketplace|list" (echo codex stdout diagnostic & echo error: unrecognized subcommand list 1>&2 & exit /b 7)',
        "plugin_list": 'if "%~1|%~2|%~3"=="plugin|list|--json" (echo codex stdout diagnostic & echo error: unrecognized subcommand list 1>&2 & exit /b 7)',
    }
    failure_line = "" if failure_command is None else failure_lines[failure_command]
    success_stderr_lines = {
        "marketplace_list": 'if "%~1|%~2|%~3"=="plugin|marketplace|list" echo warning: marketplace snapshot refreshed 1>&2',
        "plugin_list": 'if "%~1|%~2|%~3"=="plugin|list|--json" echo warning: plugin snapshot refreshed 1>&2',
    }
    success_stderr_line = (
        ""
        if success_stderr_command is None
        else success_stderr_lines[success_stderr_command]
    )
    if codex_launcher == "exe":
        fake_codex = compile_fake_codex_exe(
            repo_root,
            marketplace_json,
            plugin_json,
            large_failure_output=large_failure_output,
        )
    elif utf8_inventory_output:
        fake_codex = repo_root / "fake-codex.cmd"
        fake_codex_script = repo_root / "fake-codex.py"
        raw_marketplace_json = json.dumps(
            json.loads(marketplace_json), ensure_ascii=False
        )
        raw_plugin_json = json.dumps(json.loads(plugin_json), ensure_ascii=False)
        _ = fake_codex_script.write_text(
            textwrap.dedent(
                f"""\
                import sys

                arguments = sys.argv[1:]
                if arguments[:3] == ["plugin", "marketplace", "list"]:
                    sys.stdout.buffer.write({raw_marketplace_json!r}.encode("utf-8") + b"\\n")
                elif arguments[:3] == ["plugin", "list", "--json"]:
                    sys.stderr.buffer.write("warning: UTF-8 plugin inventory\\n".encode("utf-8"))
                    sys.stdout.buffer.write({raw_plugin_json!r}.encode("utf-8") + b"\\n")
                raise SystemExit(0)
                """
            ),
            encoding="utf-8",
        )
        _ = fake_codex.write_text(
            textwrap.dedent(
                f"""\
                @echo off
                "{sys.executable}" "%~dp0fake-codex.py" %*
                """
            ).replace("\n", "\r\n"),
            encoding="utf-8",
        )
    else:
        fake_codex = repo_root / "fake-codex.cmd"
        _ = fake_codex.write_text(
            textwrap.dedent(
                f"""\
                @echo off
                {failure_line}
                {success_stderr_line}
                if "%~1|%~2|%~3"=="plugin|marketplace|list" (
                  echo {marketplace_json}
                  exit /b 0
                )
                if "%~1|%~2|%~3"=="plugin|list|--json" (
                  echo {plugin_json}
                  exit /b 0
                )
                exit /b 0
                """
            ).replace("\n", "\r\n"),
            encoding="utf-8",
        )
    codex_executable_argument = fake_codex
    if codex_launcher == "ps1":
        codex_executable_argument = fake_codex.with_suffix(".ps1")
        _ = codex_executable_argument.write_text(
            "throw 'The installer must prefer the sibling .cmd launcher.'\n",
            encoding="utf-8",
        )
    arguments = [
        "powershell.exe",
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        str(installer),
        "-Runtime",
        "python",
        "-PythonExe",
        sys.executable if use_system_python else str(fake_python),
        "-CodexExe",
        str(codex_executable_argument),
        "-SkipDependencies",
        "-SkipEnvFile",
    ]
    if dry_run:
        arguments.append("-DryRun")
    completed = subprocess.run(
        arguments,
        cwd=repo_root,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        check=False,
    )
    if codex_launcher == "exe":
        _unlink_executable_with_retry(fake_codex)
    return completed
