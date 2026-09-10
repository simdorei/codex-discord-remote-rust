from __future__ import annotations

import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from collections.abc import Callable, Sequence
from pathlib import Path
from typing import cast

from codex_app_server_transport_resident import ResidentCodexAppServerTransport
from codex_pro_release_evidence import (
    EvidenceCheck,
    EvidenceStatus,
    ProReleaseEvidence,
    normalize_checks,
)
from codex_pro_release_checks_types import CommandOutcome, CommandRunner
from codex_pro_release_workspace import (
    git_revision,
    workspace_digest,
    workspace_state,
)
from codex_pro_runtime_preflight import (
    PLUGIN_MANIFEST_PATH,
    expected_remote_plugin_version,
    run_pro_runtime_preflight,
)
from verify_codex_plugin_inventory import InventoryVerificationError, verify_inventory

MARKETPLACE_NAME = "codex-discord-remote"
REMOTE_PLUGIN_ID = "codex-discord-remote@codex-discord-remote"


ResidentProbe = Callable[[], EvidenceStatus]
ExecutableLocator = Callable[[str], str | None]


_COMMON_SOURCE_TESTS: tuple[tuple[str, tuple[str, ...]], ...] = (
    (
        "remote_mcp_capability_contract",
        (
            "-m",
            "pytest",
            "-q",
            "tests/test_remote_mcp_capability_inventory.py",
        ),
    ),
    (
        "browser_evidence_contract",
        (
            "-m",
            "unittest",
            "tests.test_codex_discord_plugin_packaging",
            "tests.test_browser_evidence_hook",
            "tests.test_browser_evidence_probe",
        ),
    ),
    (
        "pro_runtime_contract",
        (
            "-m",
            "unittest",
            "tests.test_codex_plugin_runtime_fingerprint",
            "tests.test_codex_discord_plain_ask_preflight",
            "tests.test_codex_discord_prompt_mapped_delivery",
        ),
    ),
)


def run_command(command: Sequence[str], cwd: Path) -> CommandOutcome:
    try:
        completed = subprocess.run(
            list(command),
            cwd=cwd,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=300.0,
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        return CommandOutcome(1, "", "")
    return CommandOutcome(completed.returncode, completed.stdout, completed.stderr)


def collect_current_release_evidence(
    repo_root: Path,
    *,
    command_runner: CommandRunner = run_command,
    resident_probe: ResidentProbe | None = None,
    python_executable: str = sys.executable,
    codex_locator: ExecutableLocator = shutil.which,
) -> ProReleaseEvidence:
    root = repo_root.resolve()
    revision_before = git_revision(root, command_runner)
    workspace_digest_before = workspace_digest(root, command_runner)
    current_workspace_state = workspace_state(root, command_runner)
    checks: list[EvidenceCheck] = []

    manifest_status, plugin_version = _manifest_check(root)
    checks.append(EvidenceCheck("plugin_manifest", "source", manifest_status))
    for check_id, test_arguments in _source_tests():
        outcome = command_runner(
            (python_executable, *test_arguments),
            root,
        )
        status = classify_test_outcome(outcome)
        checks.append(EvidenceCheck(check_id, "source", status))

    checks.append(
        EvidenceCheck(
            "installed_plugin_inventory",
            "installed",
            _installed_inventory_check(root, command_runner, codex_locator),
        )
    )
    probe = resident_probe or probe_fresh_resident
    checks.append(EvidenceCheck("fresh_resident_preflight", "runtime", probe()))

    revision_after = git_revision(root, command_runner)
    workspace_digest_after = workspace_digest(root, command_runner)
    revision_status = (
        EvidenceStatus.PASSED
        if (
            revision_before
            and revision_before == revision_after
            and workspace_digest_before is not None
            and workspace_digest_before == workspace_digest_after
        )
        else EvidenceStatus.STALE
    )
    checks.append(
        EvidenceCheck("repository_revision_stable", "source", revision_status)
    )
    return ProReleaseEvidence(
        repository_revision=revision_before or "unavailable",
        workspace_state=current_workspace_state,
        host_platform=platform.system().casefold() or "unknown",
        plugin_version=plugin_version,
        checks=normalize_checks(tuple(checks)),
    )


def probe_fresh_resident() -> EvidenceStatus:
    transport = ResidentCodexAppServerTransport()
    try:
        transport.start()
        _ = run_pro_runtime_preflight(
            resident_snapshot_reader=transport.lifecycle_snapshot,
        )
    except Exception:
        return EvidenceStatus.FAILED
    finally:
        transport.close()
    return EvidenceStatus.PASSED


def _manifest_check(repo_root: Path) -> tuple[EvidenceStatus, str]:
    manifest_path = repo_root / PLUGIN_MANIFEST_PATH.relative_to(Path(__file__).parent)
    try:
        return EvidenceStatus.PASSED, expected_remote_plugin_version(manifest_path)
    except Exception:
        return EvidenceStatus.MALFORMED, "unavailable"


def _installed_inventory_check(
    repo_root: Path,
    command_runner: CommandRunner,
    codex_locator: ExecutableLocator,
) -> EvidenceStatus:
    codex_executable = codex_locator("codex")
    if codex_executable is None:
        return EvidenceStatus.SKIPPED
    marketplace = command_runner(
        (codex_executable, "plugin", "marketplace", "list", "--json"), repo_root
    )
    plugins = command_runner(
        (codex_executable, "plugin", "list", "--json"), repo_root
    )
    if marketplace.returncode != 0 or plugins.returncode != 0:
        return EvidenceStatus.FAILED
    if not _json_object(marketplace.stdout) or not _json_object(plugins.stdout):
        return EvidenceStatus.MALFORMED
    with tempfile.TemporaryDirectory() as raw_dir:
        temp_root = Path(raw_dir)
        marketplace_path = temp_root / "marketplaces.json"
        plugins_path = temp_root / "plugins.json"
        _ = marketplace_path.write_text(marketplace.stdout, encoding="utf-8")
        _ = plugins_path.write_text(plugins.stdout, encoding="utf-8")
        try:
            _ = verify_inventory(
                marketplace_inventory_path=marketplace_path,
                plugin_inventory_path=plugins_path,
                plugin_manifest_path=repo_root / "plugins/codex-discord-remote/.codex-plugin/plugin.json",
                expected_root=repo_root,
                marketplace_name=MARKETPLACE_NAME,
                plugin_id=REMOTE_PLUGIN_ID,
            )
        except InventoryVerificationError:
            return EvidenceStatus.FAILED
    return EvidenceStatus.PASSED


def _json_object(value: str) -> bool:
    try:
        parsed = cast(object, json.loads(value))
    except json.JSONDecodeError:
        return False
    return isinstance(parsed, dict)


def _source_tests() -> tuple[tuple[str, tuple[str, ...]], ...]:
    installer_module = (
        "tests.test_install_plugin_contract"
        if os.name == "nt"
        else "tests.test_install_plugin_contract_shell"
    )
    installer_check = (
        "host_installer_inventory_contract",
        ("-m", "unittest", installer_module),
    )
    return (_COMMON_SOURCE_TESTS[0], installer_check, *_COMMON_SOURCE_TESTS[1:])


def classify_test_outcome(outcome: CommandOutcome) -> EvidenceStatus:
    if outcome.returncode != 0:
        return EvidenceStatus.FAILED
    summary = (outcome.stdout + "\n" + outcome.stderr).casefold()
    if "skipped=" in summary or " skipped" in summary:
        return EvidenceStatus.SKIPPED
    return EvidenceStatus.PASSED
