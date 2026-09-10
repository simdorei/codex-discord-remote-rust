from __future__ import annotations

import json
import os
import tempfile
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import cast


SCHEMA_VERSION = 1
REQUIRED_CHECK_IDS = (
    "repository_revision_stable",
    "plugin_manifest",
    "remote_mcp_capability_contract",
    "host_installer_inventory_contract",
    "browser_evidence_contract",
    "pro_runtime_contract",
    "installed_plugin_inventory",
    "fresh_resident_preflight",
)
DEFERRED_CHECK_IDS = (
    "in_app_browser_live_evidence",
    "chatgpt_tool_exposure",
    "post_restart_runtime",
    "other_platform_installer_contract",
)


class EvidenceStatus(StrEnum):
    PASSED = "passed"
    FAILED = "failed"
    SKIPPED = "skipped"
    STALE = "stale"
    MALFORMED = "malformed"


@dataclass(frozen=True, slots=True)
class EvidenceCheck:
    check_id: str
    category: str
    status: EvidenceStatus


@dataclass(frozen=True, slots=True)
class ProReleaseEvidence:
    repository_revision: str
    workspace_state: str
    host_platform: str
    plugin_version: str
    checks: tuple[EvidenceCheck, ...]

    @property
    def pre_restart_ready(self) -> bool:
        return all(check.status == EvidenceStatus.PASSED for check in self.checks)

    def to_payload(self) -> dict[str, object]:
        return {
            "schema_version": SCHEMA_VERSION,
            "certification_scope": "pre_restart",
            "repository_revision": self.repository_revision,
            "workspace_state": self.workspace_state,
            "host_platform": self.host_platform,
            "plugin_version": self.plugin_version,
            "required_check_ids": list(REQUIRED_CHECK_IDS),
            "checks": [
                {
                    "id": check.check_id,
                    "category": check.category,
                    "status": check.status.value,
                }
                for check in self.checks
            ],
            "pre_restart_ready": self.pre_restart_ready,
            "release_ready": False,
            "deferred_check_ids": list(DEFERRED_CHECK_IDS),
        }

    def summary(self) -> str:
        headline = "PRE-RESTART READY" if self.pre_restart_ready else "PRE-RESTART BLOCKED"
        lines = [headline]
        lines.extend(
            f"{check.status.value.upper():9} {check.check_id}"
            for check in self.checks
        )
        lines.append("DEFERRED  " + ", ".join(DEFERRED_CHECK_IDS))
        lines.append("RELEASE READY: NO (live Browser, ChatGPT exposure, and restart remain)")
        return "\n".join(lines) + "\n"


def normalize_checks(checks: tuple[EvidenceCheck, ...]) -> tuple[EvidenceCheck, ...]:
    by_id: dict[str, list[EvidenceCheck]] = {}
    for check in checks:
        by_id.setdefault(check.check_id, []).append(check)
    normalized: list[EvidenceCheck] = []
    for check_id in REQUIRED_CHECK_IDS:
        matches = by_id.get(check_id, [])
        if not matches:
            normalized.append(EvidenceCheck(check_id, "contract", EvidenceStatus.SKIPPED))
        elif len(matches) != 1:
            normalized.append(EvidenceCheck(check_id, "contract", EvidenceStatus.MALFORMED))
        else:
            normalized.append(matches[0])
    if set(by_id).difference(REQUIRED_CHECK_IDS):
        normalized[0] = EvidenceCheck(
            REQUIRED_CHECK_IDS[0], "contract", EvidenceStatus.MALFORMED
        )
    return tuple(normalized)


def write_evidence_artifacts(
    evidence: ProReleaseEvidence,
    json_path: Path,
) -> tuple[Path, Path]:
    summary_path = json_path.with_suffix(".txt")
    payload = json.dumps(
        evidence.to_payload(),
        ensure_ascii=False,
        indent=2,
        sort_keys=True,
    ) + "\n"
    _atomic_write(json_path, payload)
    _atomic_write(summary_path, evidence.summary())
    return json_path, summary_path


def read_evidence(path: Path) -> dict[str, object]:
    value = cast(object, json.loads(path.read_text(encoding="utf-8")))
    if not isinstance(value, dict):
        raise ValueError("release evidence must be a JSON object")
    return cast("dict[str, object]", value)


def _atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, raw_temp_path = tempfile.mkstemp(
        prefix=f".{path.name}.",
        suffix=".tmp",
        dir=path.parent,
    )
    temp_path = Path(raw_temp_path)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as target:
            _ = target.write(content)
            target.flush()
            os.fsync(target.fileno())
        _ = temp_path.replace(path)
    finally:
        _ = temp_path.unlink(missing_ok=True)
