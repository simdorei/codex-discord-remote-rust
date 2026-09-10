from __future__ import annotations

import json
import shlex
import stat
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Literal

from tests.install_plugin_inventory_cases import EXPECTED_VERSION

ROOT = Path(__file__).resolve().parents[1]
FailureCommand = Literal[
    "marketplace_add",
    "plugin_add",
    "marketplace_list",
    "plugin_list",
]


def prepare_installer_repo(repo_root: Path, installer_name: str) -> Path:
    installer = repo_root / installer_name
    _ = installer.write_text(
        (ROOT / installer_name).read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    runtime_release = ROOT / "runtime-release.json"
    _ = (repo_root / runtime_release.name).write_text(
        runtime_release.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    marketplace = repo_root / ".agents" / "plugins" / "marketplace.json"
    marketplace.parent.mkdir(parents=True)
    _ = marketplace.write_text("{}\n", encoding="utf-8")
    verifier_source = ROOT / "verify_codex_plugin_inventory.py"
    _ = (repo_root / verifier_source.name).write_text(
        verifier_source.read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    plugin_manifest = (
        repo_root / "plugins/codex-discord-remote/.codex-plugin/plugin.json"
    )
    plugin_manifest.parent.mkdir(parents=True)
    _ = plugin_manifest.write_text(
        json.dumps({"version": EXPECTED_VERSION}) + "\n",
        encoding="utf-8",
    )
    return installer


def run_shell_installer(
    repo_root: Path,
    marketplace_json: str,
    plugin_json: str,
    *,
    failure_command: FailureCommand | None = None,
    dry_run: bool = False,
) -> subprocess.CompletedProcess[str]:
    installer = prepare_installer_repo(repo_root, "install.sh")
    installer.chmod(installer.stat().st_mode | stat.S_IXUSR)
    fake_python = repo_root / "fake-python"
    _ = fake_python.write_text(
        textwrap.dedent(
            f"""\
            #!/usr/bin/env sh
            if [ "${{1##*/}}" = "verify_codex_plugin_inventory.py" ]; then
              exec {shlex.quote(sys.executable)} "$@"
            fi
            exit 0
            """
        ),
        encoding="utf-8",
    )
    fake_python.chmod(fake_python.stat().st_mode | stat.S_IXUSR)
    failure_lines = {
        "marketplace_add": '[ "$1|$2|${3:-}" = "plugin|marketplace|add" ] && exit 7',
        "plugin_add": '[ "$1|$2" = "plugin|add" ] && exit 7',
        "marketplace_list": '[ "$1|$2|${3:-}" = "plugin|marketplace|list" ] && exit 7',
        "plugin_list": '[ "$1|$2|${3:-}" = "plugin|list|--json" ] && exit 7',
    }
    failure_line = "" if failure_command is None else failure_lines[failure_command]
    fake_codex = repo_root / "fake-codex"
    _ = fake_codex.write_text(
        textwrap.dedent(
            f"""\
            #!/usr/bin/env sh
            {failure_line}
            if [ "$1|$2|${{3:-}}" = "plugin|marketplace|list" ]; then
              printf '%s\\n' {shlex.quote(marketplace_json)}
              exit 0
            fi
            if [ "$1|$2|${{3:-}}" = "plugin|list|--json" ]; then
              printf '%s\\n' {shlex.quote(plugin_json)}
              exit 0
            fi
            exit 0
            """
        ),
        encoding="utf-8",
    )
    fake_codex.chmod(fake_codex.stat().st_mode | stat.S_IXUSR)
    arguments = [
        "sh",
        str(installer),
        "--python-exe",
        str(fake_python),
        "--codex-exe",
        str(fake_codex),
        "--skip-dependencies",
        "--skip-env-file",
    ]
    if dry_run:
        arguments.append("--dry-run")
    return subprocess.run(
        arguments,
        capture_output=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        check=False,
    )
