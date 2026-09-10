from __future__ import annotations

import hashlib
import json
import os
import re
import sys
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import Literal, TypeAlias, assert_never, cast
from uuid import uuid4


PROTOCOL = "ask-chatgpt-pro-connector-control-v1"
CONNECTOR_NAME = "Simdorei Local Project Oauth"
CONNECTOR_PATH = "/plugins/plugin_asdk_app_6a6ae90be0a08191b877eddba93b631c"
EXPECTED_PROBE_SHA256 = (
    "c7bccace3ffed083682369ac25a8ba6ab09aa3f64c238010a7a8ceef40c5e36f"
)
PLUGIN_ROOT = Path(__file__).resolve().parent.parent
PROBE_RELATIVE_PATH = Path(
    "skills/ask-chatgpt-pro/scripts/pro_connector_control.mjs"
)
EXEC_TOOL_NAMES = {"exec", "functions.exec"}
NODE_TOOL_NAMES = {"js", "mcp__node_repl__js", "node_repl.js"}
FailureStage: TypeAlias = Literal[
    "plugin_data_missing",
    "session_id_missing",
    "turn_id_missing",
    "write_failed",
]
JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]
OuterResultIdentifier: TypeAlias = Literal[
    "connectorControlResult", "connectorControlRetryResult"
]


def canonical_inner_probe_code(plugin_root: Path | None = None) -> str:
    root = plugin_root or PLUGIN_ROOT
    probe_uri = (root / PROBE_RELATIVE_PATH).resolve().as_uri()
    encoded_uri = json.dumps(probe_uri, ensure_ascii=True)
    return (
        "nodeRepl.write(JSON.stringify(await (await import("
        f"{encoded_uri})).prepareProConnector(globalThis)));"
    )


def _canonical_outer_probe_code(
    plugin_root: Path | None, result_identifier: OuterResultIdentifier
) -> str:
    match result_identifier:
        case "connectorControlResult" | "connectorControlRetryResult":
            pass
        case unreachable:
            assert_never(unreachable)
    tool_input = json.dumps(
        {
            "code": canonical_inner_probe_code(plugin_root),
            "title": "Prepare Pro OAuth connector",
        },
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return (
        f"const {result_identifier} = await "
        f"tools.mcp__node_repl__js({tool_input});\n"
        f"for (const item of {result_identifier}.content ?? []) {{\n"
        '  if (item.type === "text") text(item.text);\n'
        "}"
    )


def canonical_probe_code(plugin_root: Path | None = None) -> str:
    return _canonical_outer_probe_code(plugin_root, "connectorControlResult")


def canonical_retry_probe_code(plugin_root: Path | None = None) -> str:
    return _canonical_outer_probe_code(plugin_root, "connectorControlRetryResult")


def canonical_probe_codes(plugin_root: Path | None = None) -> tuple[str, str]:
    return canonical_probe_code(plugin_root), canonical_retry_probe_code(plugin_root)


def process_post_tool_use(
    payload: Mapping[str, JsonValue],
    plugin_data: Path | None = None,
    plugin_root: Path | None = None,
) -> bool:
    if payload.get("hook_event_name") != "PostToolUse":
        return False
    tool_name = payload.get("tool_name")
    if not isinstance(tool_name, str):
        return False
    root = plugin_root or PLUGIN_ROOT
    expected = _expected_codes(tool_name, root)
    accepted = {
        variant
        for code in expected
        for variant in (code, f"{code}\n", f"{code}\r\n")
    }
    if _tool_code(payload.get("tool_input")) not in accepted:
        return False
    if not _probe_integrity_ok(root):
        return False
    session_id = payload.get("session_id")
    turn_id = payload.get("turn_id")
    if not isinstance(session_id, str) or not session_id:
        _report_failure("session_id_missing")
        return False
    if not isinstance(turn_id, str) or not turn_id:
        _report_failure("turn_id_missing")
        return False
    evidence = _single_valid_evidence(payload.get("tool_response"))
    if evidence is None:
        evidence = {
            "protocol": PROTOCOL,
            "browser_type": "chrome",
            "status": "failed",
            "connector_name": CONNECTOR_NAME,
            "connector_path": CONNECTOR_PATH,
            "chat_mode": "unverified",
            "pro_mode": False,
            "action": "none",
            "failed_stage": "evidence_invalid",
        }
    receipt = {
        **evidence,
        "session_id": session_id,
        "turn_id": turn_id,
        "probe_sha256": EXPECTED_PROBE_SHA256,
    }
    return _write_receipt(receipt, plugin_data)


def _expected_codes(tool_name: str, plugin_root: Path) -> tuple[str, ...]:
    if tool_name in EXEC_TOOL_NAMES:
        return canonical_probe_codes(plugin_root)
    if tool_name in NODE_TOOL_NAMES:
        return (canonical_inner_probe_code(plugin_root),)
    return ()


def _tool_code(raw: JsonValue) -> str | None:
    if isinstance(raw, str):
        return raw
    values = _object_map(raw)
    if values is None:
        return None
    for key in ("code", "input"):
        value = values.get(key)
        if isinstance(value, str):
            return value
    return None


def _probe_integrity_ok(plugin_root: Path) -> bool:
    try:
        source = (plugin_root / PROBE_RELATIVE_PATH).read_bytes()
    except OSError:
        return False
    canonical = source.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    return hashlib.sha256(canonical).hexdigest() == EXPECTED_PROBE_SHA256


def _single_valid_evidence(raw: JsonValue) -> JsonObject | None:
    decoder = json.JSONDecoder()
    candidates: list[JsonObject] = []
    for text in _string_values(raw):
        for match in re.finditer(r"\{", text):
            try:
                value, _ = cast(
                    tuple[JsonValue, int], decoder.raw_decode(text[match.start() :])
                )
            except json.JSONDecodeError:
                continue
            evidence = _object_map(value)
            if evidence is not None and _valid_evidence(evidence):
                candidates.append(evidence)
    return candidates[0] if len(candidates) == 1 else None


def _valid_evidence(evidence: Mapping[str, JsonValue]) -> bool:
    if not (
        evidence.get("protocol") == PROTOCOL
        and evidence.get("browser_type") == "chrome"
        and evidence.get("connector_name") == CONNECTOR_NAME
        and evidence.get("connector_path") == CONNECTOR_PATH
    ):
        return False
    status = evidence.get("status")
    failed_stage = evidence.get("failed_stage")
    verified = (
        status == "verified"
        and evidence.get("chat_mode") == "chat"
        and evidence.get("pro_mode") is True
        and evidence.get("action") in {"attached", "already_attached"}
        and "failed_stage" not in evidence
    )
    failed = (
        status == "failed"
        and evidence.get("chat_mode") == "unverified"
        and evidence.get("pro_mode") is False
        and evidence.get("action") == "none"
        and isinstance(failed_stage, str)
        and bool(failed_stage)
    )
    return verified or failed


def _string_values(raw: JsonValue) -> Iterable[str]:
    if isinstance(raw, str):
        yield raw
        return
    values = _object_map(raw)
    if values is not None:
        for value in values.values():
            yield from _string_values(value)
        return
    if isinstance(raw, list):
        for value in raw:
            yield from _string_values(value)


def _object_map(raw: JsonValue) -> JsonObject | None:
    return raw if isinstance(raw, dict) else None


def _write_receipt(receipt: Mapping[str, JsonValue], plugin_data: Path | None) -> bool:
    data_path = plugin_data or _environment_data_path()
    if data_path is None:
        _report_failure("plugin_data_missing")
        return False
    session_id = receipt.get("session_id")
    turn_id = receipt.get("turn_id")
    if not isinstance(session_id, str) or not isinstance(turn_id, str):
        return False
    key = hashlib.sha256(f"{session_id}\0{turn_id}".encode()).hexdigest()
    path = data_path / "pro-connector-evidence" / f"{key}.json"
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f".{path.name}.{uuid4().hex}.tmp")
        _ = temporary.write_text(json.dumps(dict(receipt)), encoding="utf-8")
        _ = temporary.replace(path)
    except OSError:
        _report_failure("write_failed")
        return False
    return True


def _report_failure(stage: FailureStage) -> None:
    _ = sys.stderr.write(f"pro_connector_evidence_hook_failed stage={stage}\n")


def _environment_data_path() -> Path | None:
    raw = os.environ.get("PLUGIN_DATA")
    return Path(raw) if raw else None


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "print-probe-code":
        _ = sys.stdout.write(canonical_probe_code())
        return 0
    if len(sys.argv) == 2 and sys.argv[1] == "print-retry-probe-code":
        _ = sys.stdout.write(canonical_retry_probe_code())
        return 0
    try:
        payload = cast(JsonValue, json.loads(sys.stdin.read()))
    except json.JSONDecodeError:
        return 0
    values = _object_map(payload)
    if values is not None:
        _ = process_post_tool_use(values)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
