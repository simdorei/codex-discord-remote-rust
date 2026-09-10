from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Final, cast

import codex_pro_connector_transcript as connector_transcript

PROTOCOL: Final = connector_transcript.PROTOCOL
CONNECTOR_NAME: Final = connector_transcript.CONNECTOR_NAME
CONNECTOR_PATH: Final = connector_transcript.CONNECTOR_PATH
EXPECTED_PROBE_SHA256: Final = connector_transcript.EXPECTED_PROBE_SHA256
PLUGIN_DATA_DIRECTORY: Final = "codex-discord-remote-codex-discord-remote"


class ProConnectorUnavailableError(RuntimeError):
    def __init__(self, internal_detail: str) -> None:
        super().__init__("pro_connector_unavailable")
        self.internal_detail = internal_detail


def default_plugin_data_path() -> Path:
    configured_home = os.environ.get("CODEX_HOME", "").strip()
    codex_home = Path(configured_home) if configured_home else Path.home() / ".codex"
    return codex_home / "plugins" / "data" / PLUGIN_DATA_DIRECTORY


def require_verified_evidence(
    session_id: str,
    turn_id: str,
    *,
    plugin_data: Path | None = None,
) -> None:
    data_path = plugin_data or default_plugin_data_path()
    receipt_path = _receipt_path(session_id, turn_id, data_path)
    transcript_result = connector_transcript.read_transcript_evidence(
        session_id,
        turn_id,
        connector_transcript.default_evidence_source(),
    )
    if receipt_path.exists():
        receipt_evidence = _read_receipt(session_id, turn_id, receipt_path)
        if receipt_evidence is None:
            raise ProConnectorUnavailableError(
                "Exact-turn connector receipt exists but is invalid or unreadable."
            )
        evidence = (
            transcript_result.evidence
            if transcript_result.trusted_attempt_seen
            else receipt_evidence
        )
    else:
        evidence = transcript_result.evidence
    if evidence is None:
        raise ProConnectorUnavailableError(
            "Pro turn completed without exact-turn connector control evidence."
        )
    if evidence.get("status") != "verified":
        stage = evidence.get("failed_stage")
        raise ProConnectorUnavailableError(
            f"Chrome connector control was not verified: stage={stage or 'unknown'}"
        )
    expected = {
        "connector_name": CONNECTOR_NAME,
        "connector_path": CONNECTOR_PATH,
        "chat_mode": "chat",
        "pro_mode": True,
    }
    if any(evidence.get(key) != value for key, value in expected.items()):
        raise ProConnectorUnavailableError(
            "Connector, Chat mode, or Pro mode evidence did not match the contract."
        )
    if (
        evidence.get("action") not in {"attached", "already_attached"}
        or "failed_stage" in evidence
    ):
        raise ProConnectorUnavailableError(
            "Verified connector evidence had an invalid action or failure stage."
        )


def _receipt_path(session_id: str, turn_id: str, plugin_data: Path) -> Path:
    key = hashlib.sha256(f"{session_id}\0{turn_id}".encode()).hexdigest()
    return plugin_data / "pro-connector-evidence" / f"{key}.json"


def _read_receipt(
    session_id: str,
    turn_id: str,
    path: Path,
) -> connector_transcript.JsonObject | None:
    if not session_id or not turn_id:
        return None
    try:
        raw = cast(
            connector_transcript.JsonValue,
            json.loads(path.read_text(encoding="utf-8")),
        )
    except (OSError, json.JSONDecodeError):
        return None
    evidence = _object_map(raw)
    if evidence is None:
        return None
    if (
        evidence.get("protocol") != PROTOCOL
        or evidence.get("session_id") != session_id
        or evidence.get("turn_id") != turn_id
        or evidence.get("browser_type") != "chrome"
        or evidence.get("probe_sha256") != EXPECTED_PROBE_SHA256
        or not connector_transcript.is_valid_evidence(evidence)
    ):
        return None
    return evidence


def _object_map(
    raw: connector_transcript.JsonValue,
) -> connector_transcript.JsonObject | None:
    return raw if isinstance(raw, dict) else None
