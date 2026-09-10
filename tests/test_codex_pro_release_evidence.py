from __future__ import annotations

import json
import tempfile
import unittest
from collections.abc import Sequence
from pathlib import Path
from typing import cast

from codex_pro_release_checks import (
    classify_test_outcome,
    collect_current_release_evidence,
)
from codex_pro_release_checks_types import CommandOutcome
from codex_pro_release_evidence import (
    DEFERRED_CHECK_IDS,
    REQUIRED_CHECK_IDS,
    EvidenceCheck,
    EvidenceStatus,
    ProReleaseEvidence,
    normalize_checks,
    read_evidence,
    write_evidence_artifacts,
)
from codex_pro_runtime_preflight import expected_remote_plugin_version


class ProReleaseEvidenceTests(unittest.TestCase):
    def test_artifact_is_deterministic_ordered_and_public_safe(self) -> None:
        checks = tuple(
            EvidenceCheck(check_id, "source", EvidenceStatus.PASSED)
            for check_id in reversed(REQUIRED_CHECK_IDS)
        )
        evidence = ProReleaseEvidence(
            repository_revision="revision-1",
            workspace_state="dirty",
            host_platform="windows",
            plugin_version="1.2.3",
            checks=normalize_checks(checks),
        )

        with tempfile.TemporaryDirectory() as raw_dir:
            first = Path(raw_dir) / "first.json"
            second = Path(raw_dir) / "second.json"
            _ = write_evidence_artifacts(evidence, first)
            _ = write_evidence_artifacts(evidence, second)
            first_text = first.read_text(encoding="utf-8")
            second_text = second.read_text(encoding="utf-8")
            payload = read_evidence(first)

        self.assertEqual(first_text, second_text)
        raw_checks = payload["checks"]
        self.assertIsInstance(raw_checks, list)
        payload_checks = cast("list[dict[str, object]]", raw_checks)
        self.assertEqual(
            [check["id"] for check in payload_checks],
            list(REQUIRED_CHECK_IDS),
        )
        self.assertTrue(payload["pre_restart_ready"])
        self.assertFalse(payload["release_ready"])
        self.assertEqual(payload["deferred_check_ids"], list(DEFERRED_CHECK_IDS))
        for forbidden in (
            "source.path",
            "fingerprint",
            "project_scope",
            "conversation_scope",
            "token",
            "C:/private",
        ):
            self.assertNotIn(forbidden, first_text)

    def test_every_nonpassing_required_status_fails_closed(self) -> None:
        for status in (
            EvidenceStatus.FAILED,
            EvidenceStatus.SKIPPED,
            EvidenceStatus.STALE,
            EvidenceStatus.MALFORMED,
        ):
            with self.subTest(status=status):
                checks = [
                    EvidenceCheck(check_id, "source", EvidenceStatus.PASSED)
                    for check_id in REQUIRED_CHECK_IDS
                ]
                checks[3] = EvidenceCheck(
                    REQUIRED_CHECK_IDS[3], "source", status
                )
                evidence = ProReleaseEvidence(
                    "revision-1",
                    "clean",
                    "windows",
                    "1.2.3",
                    tuple(checks),
                )
                self.assertFalse(evidence.pre_restart_ready)

    def test_missing_duplicate_and_unexpected_checks_are_not_ready(self) -> None:
        complete = [
            EvidenceCheck(check_id, "source", EvidenceStatus.PASSED)
            for check_id in REQUIRED_CHECK_IDS
        ]
        cases = (
            tuple(complete[1:]),
            tuple([complete[0], *complete]),
            tuple(
                [
                    *complete,
                    EvidenceCheck("unexpected", "source", EvidenceStatus.PASSED),
                ]
            ),
        )
        for checks in cases:
            with self.subTest(check_count=len(checks)):
                normalized = normalize_checks(checks)
                evidence = ProReleaseEvidence(
                    "revision-1", "clean", "windows", "1.2.3", normalized
                )
                self.assertFalse(evidence.pre_restart_ready)


class ProReleaseCheckCollectionTests(unittest.TestCase):
    def test_collector_runs_source_installed_and_fresh_resident_checks(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        version = expected_remote_plugin_version()
        calls: list[tuple[str, ...]] = []

        def runner(command: Sequence[str], cwd: Path) -> CommandOutcome:
            self.assertEqual(cwd, repo_root)
            parts = tuple(command)
            calls.append(parts)
            joined = " ".join(parts)
            if "rev-parse HEAD" in joined:
                return CommandOutcome(0, "revision-1\n", "")
            if "status --porcelain" in joined:
                return CommandOutcome(0, " M C:/private/project_scope-token\n", "")
            if "diff --binary" in joined or "ls-files --others" in joined:
                return CommandOutcome(0, "", "")
            if "marketplace list --json" in joined:
                return CommandOutcome(
                    0,
                    json.dumps(
                        {
                            "marketplaces": [
                                {
                                    "name": "codex-discord-remote",
                                    "root": str(repo_root),
                                }
                            ]
                        }
                    ),
                    "",
                )
            if "plugin list --json" in joined:
                return CommandOutcome(
                    0,
                    json.dumps(
                        {
                            "installed": [
                                {
                                    "pluginId": (
                                        "codex-discord-remote@codex-discord-remote"
                                    ),
                                    "installed": True,
                                    "enabled": True,
                                    "version": version,
                                    "source": {"path": "C:/private/plugin"},
                                }
                            ]
                        }
                    ),
                    "",
                )
            return CommandOutcome(0, "", "")

        evidence = collect_current_release_evidence(
            repo_root,
            command_runner=runner,
            resident_probe=lambda: EvidenceStatus.PASSED,
            python_executable="python-for-test",
            codex_locator=lambda _: "codex-for-test",
        )

        self.assertTrue(evidence.pre_restart_ready)
        self.assertEqual(evidence.workspace_state, "dirty")
        self.assertEqual(evidence.repository_revision, "revision-1")
        self.assertEqual(len(calls), 13)
        serialized = json.dumps(evidence.to_payload())
        self.assertNotIn("C:/private", serialized)
        self.assertNotIn("project_scope", serialized)
        self.assertNotIn("source.path", serialized)

    def test_malformed_inventory_and_revision_change_fail_closed(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        revisions = iter(("revision-1\n", "revision-2\n"))

        def runner(command: Sequence[str], cwd: Path) -> CommandOutcome:
            _ = cwd
            parts = tuple(command)
            joined = " ".join(parts)
            if "rev-parse HEAD" in joined:
                return CommandOutcome(0, next(revisions), "")
            if "status --porcelain" in joined:
                return CommandOutcome(0, "", "")
            if "diff --binary" in joined or "ls-files --others" in joined:
                return CommandOutcome(0, "", "")
            if "marketplace list --json" in joined:
                return CommandOutcome(0, "not-json", "")
            if "plugin list --json" in joined:
                return CommandOutcome(0, "{}", "")
            return CommandOutcome(0, "", "")

        evidence = collect_current_release_evidence(
            repo_root,
            command_runner=runner,
            resident_probe=lambda: EvidenceStatus.PASSED,
            codex_locator=lambda _: "codex-for-test",
        )
        by_id = {check.check_id: check.status for check in evidence.checks}

        self.assertEqual(
            by_id["installed_plugin_inventory"], EvidenceStatus.MALFORMED
        )
        self.assertEqual(
            by_id["repository_revision_stable"], EvidenceStatus.STALE
        )
        self.assertFalse(evidence.pre_restart_ready)

    def test_workspace_change_with_same_head_is_stale(self) -> None:
        repo_root = Path(__file__).resolve().parents[1]
        diffs = iter(("before", "after"))

        def runner(command: Sequence[str], cwd: Path) -> CommandOutcome:
            _ = cwd
            joined = " ".join(command)
            if "rev-parse HEAD" in joined:
                return CommandOutcome(0, "revision-1\n", "")
            if "diff --binary" in joined:
                return CommandOutcome(0, next(diffs), "")
            if "ls-files --others" in joined or "status --porcelain" in joined:
                return CommandOutcome(0, "", "")
            if "marketplace list --json" in joined:
                return CommandOutcome(1, "", "")
            if "plugin list --json" in joined:
                return CommandOutcome(1, "", "")
            return CommandOutcome(0, "", "")

        evidence = collect_current_release_evidence(
            repo_root,
            command_runner=runner,
            resident_probe=lambda: EvidenceStatus.PASSED,
        )
        by_id = {check.check_id: check.status for check in evidence.checks}

        self.assertEqual(
            by_id["repository_revision_stable"], EvidenceStatus.STALE
        )

    def test_required_test_skip_is_not_reported_as_passed(self) -> None:
        status = classify_test_outcome(
            CommandOutcome(0, "OK (skipped=4)\n", "")
        )

        self.assertEqual(status, EvidenceStatus.SKIPPED)


if __name__ == "__main__":
    _ = unittest.main()
