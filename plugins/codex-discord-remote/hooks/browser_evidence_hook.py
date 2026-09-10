from __future__ import annotations

import hashlib
import json
import os
import re
import sys
from collections.abc import Iterable, Mapping
from pathlib import Path
from typing import cast
from uuid import uuid4


PROTOCOL = "ask-chatgpt-pro-browser-evidence-v2"
EXPECTED_PROBE_SHA256 = (
    "346dfe09965e91f6b4339e6c1fcfb14a76c204ef3972779780847d2f7b358853"
)
PLUGIN_ROOT = Path(__file__).resolve().parent.parent
PROBE_RELATIVE_PATH = Path(
    "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
)
EXEC_TOOL_NAMES = {"exec", "functions.exec"}
NODE_TOOL_NAMES = {"js", "mcp__node_repl__js", "node_repl.js"}
UNVERIFIED_PHRASE = "chrome bootstrap was not verified"
BLOCK_REASON = (
    "Unsupported Chrome unavailability claim blocked. Run the trusted same-turn "
    + "Chrome evidence probe, or say exactly: Chrome bootstrap was not verified."
)
ENGLISH_CLAIM = re.compile(
    r"(?:\bgoogle\s+chrome\b|\bchrome\b).{0,24}"
    + r"(?:\bunavailable\b|\bnot\s+available\b|\bcannot\s+be\s+used\b|"
    + r"\bcan['\u2019]?t\s+be\s+used\b)",
    re.IGNORECASE | re.DOTALL,
)
KOREAN_CLAIM = re.compile(
    r"(?:\uad6c\uae00\s*)?\ud06c\ub86c.{0,30}"
    + r"(?:\uc0ac\uc6a9\s*\ubd88\uac00|\uc0ac\uc6a9\ud560\s*\uc218\s*\uc5c6|"
    + r"\uc5f0\uacb0\s*\ubd88\uac00|\uc791\ub3d9\ud558\uc9c0\s*\uc54a|"
    + r"\uc548\s*(?:\ub3fc|\ub428)|\ubd88\uac00\ub2a5)",
    re.DOTALL,
)


def canonical_inner_probe_code(plugin_root: Path = PLUGIN_ROOT) -> str:
    probe_uri = (plugin_root / PROBE_RELATIVE_PATH).resolve().as_uri()
    encoded_uri = json.dumps(probe_uri, ensure_ascii=True)
    return (
        "nodeRepl.write(JSON.stringify(await (await import("
        f"{encoded_uri})).probeChrome(globalThis)));"
    )


def canonical_probe_code(plugin_root: Path = PLUGIN_ROOT) -> str:
    tool_input = json.dumps(
        {
            "code": canonical_inner_probe_code(plugin_root),
            "title": "Verify Chrome",
        },
        ensure_ascii=True,
        separators=(",", ":"),
    )
    return (
        "const browserEvidenceResult = await "
        f"tools.mcp__node_repl__js({tool_input});\n"
        "for (const item of browserEvidenceResult.content ?? []) {\n"
        '  if (item.type === "text") text(item.text);\n'
        "}"
    )


def process_post_tool_use(
    payload: Mapping[str, object],
    plugin_data: Path | None = None,
    plugin_root: Path = PLUGIN_ROOT,
) -> bool:
    if payload.get("hook_event_name") != "PostToolUse":
        return False
    tool_name = payload.get("tool_name")
    if not isinstance(tool_name, str):
        return False
    expected = _expected_code(tool_name, plugin_root)
    tool_code = _tool_code(payload.get("tool_input"))
    accepted_codes = (
        {expected, f"{expected}\n", f"{expected}\r\n"}
        if expected is not None
        else set()
    )
    if tool_code not in accepted_codes:
        return False
    if not _probe_integrity_ok(plugin_root):
        return False
    evidence = _find_valid_evidence(payload.get("tool_response"))
    if evidence is None:
        return False
    session_id = payload.get("session_id")
    turn_id = payload.get("turn_id")
    tool_use_id = payload.get("tool_use_id")
    if not isinstance(session_id, str) or not session_id:
        return False
    if not isinstance(turn_id, str) or not turn_id:
        return False
    receipt = {
        "protocol": PROTOCOL,
        "session_id": session_id,
        "turn_id": turn_id,
        "tool_use_id": tool_use_id if isinstance(tool_use_id, str) else "",
        "browser_type": evidence["browser_type"],
        "status": evidence["status"],
        "can_report_unavailable": evidence["can_report_unavailable"],
        "probe_sha256": EXPECTED_PROBE_SHA256,
    }
    return _write_receipt(receipt, plugin_data)


def process_stop(
    payload: Mapping[str, object], plugin_data: Path | None = None
) -> dict[str, str] | None:
    if payload.get("hook_event_name") != "Stop":
        return None
    message = payload.get("last_assistant_message")
    if not isinstance(message, str) or not _claims_browser_unavailable(message):
        return None
    session_id = payload.get("session_id")
    turn_id = payload.get("turn_id")
    if not isinstance(session_id, str) or not isinstance(turn_id, str):
        return {"decision": "block", "reason": BLOCK_REASON}
    receipt = _read_receipt(session_id, turn_id, plugin_data)
    if receipt is not None and _receipt_authorizes_unavailable(receipt):
        return None
    return {"decision": "block", "reason": BLOCK_REASON}


def _expected_code(tool_name: str, plugin_root: Path) -> str | None:
    if tool_name in EXEC_TOOL_NAMES:
        return canonical_probe_code(plugin_root)
    if tool_name in NODE_TOOL_NAMES:
        return canonical_inner_probe_code(plugin_root)
    return None


def _tool_code(raw: object) -> str | None:
    if isinstance(raw, str):
        return raw
    values = _object_map(raw)
    if values is not None:
        for key in ("code", "input"):
            value = values.get(key)
            if isinstance(value, str):
                return value
    return None


def _object_map(raw: object) -> dict[str, object] | None:
    if not isinstance(raw, dict):
        return None
    values = cast(dict[object, object], raw)
    if not all(isinstance(key, str) for key in values):
        return None
    return {cast(str, key): value for key, value in values.items()}


def _probe_integrity_ok(plugin_root: Path) -> bool:
    probe = plugin_root / PROBE_RELATIVE_PATH
    try:
        source = probe.read_bytes()
    except OSError:
        return False
    canonical_source = source.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    digest = hashlib.sha256(canonical_source).hexdigest()
    return digest == EXPECTED_PROBE_SHA256


def _find_valid_evidence(raw: object) -> dict[str, object] | None:
    for text in _string_values(raw):
        decoder = json.JSONDecoder()
        for match in re.finditer(r"\{", text):
            try:
                decoded = cast(
                    tuple[object, int], decoder.raw_decode(text[match.start() :])
                )
            except json.JSONDecodeError:
                continue
            value = _object_map(decoded[0])
            if value is not None and _valid_evidence(value):
                return value
    return None


def _string_values(raw: object) -> Iterable[str]:
    if isinstance(raw, str):
        yield raw
        return
    values = _object_map(raw)
    if values is not None:
        for value in values.values():
            yield from _string_values(value)
        return
    if isinstance(raw, list):
        for value in cast(list[object], raw):
            yield from _string_values(value)


def _valid_evidence(value: Mapping[str, object]) -> bool:
    if value.get("protocol") != PROTOCOL or value.get("browser_type") != "chrome":
        return False
    status = value.get("status")
    can_report = value.get("can_report_unavailable")
    if status not in {"available", "unavailable", "unverified"}:
        return False
    if status != "unavailable":
        return can_report is False
    return (
        can_report is True
        and value.get("failed_stage") == "select_chrome_retry"
        and isinstance(value.get("public_error"), str)
        and bool(value["public_error"])
    )


def _claims_browser_unavailable(message: str) -> bool:
    normalized = message.strip().casefold().removesuffix(".")
    if normalized == UNVERIFIED_PHRASE:
        return False
    return bool(ENGLISH_CLAIM.search(message) or KOREAN_CLAIM.search(message))


def _receipt_path(session_id: str, turn_id: str, plugin_data: Path) -> Path:
    key = hashlib.sha256(f"{session_id}\0{turn_id}".encode()).hexdigest()
    return plugin_data / "browser-evidence" / f"{key}.json"


def _data_path(plugin_data: Path | None) -> Path | None:
    if plugin_data is not None:
        return plugin_data
    raw = os.environ.get("PLUGIN_DATA")
    return Path(raw) if raw else None


def _write_receipt(receipt: Mapping[str, object], plugin_data: Path | None) -> bool:
    data_path = _data_path(plugin_data)
    if data_path is None:
        return False
    session_id = receipt.get("session_id")
    turn_id = receipt.get("turn_id")
    if not isinstance(session_id, str) or not isinstance(turn_id, str):
        return False
    path = _receipt_path(session_id, turn_id, data_path)
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_name(f".{path.name}.{uuid4().hex}.tmp")
        _ = temporary.write_text(json.dumps(receipt), encoding="utf-8")
        _ = temporary.replace(path)
    except OSError:
        return False
    return True


def _read_receipt(
    session_id: str, turn_id: str, plugin_data: Path | None
) -> dict[str, object] | None:
    data_path = _data_path(plugin_data)
    if data_path is None:
        return None
    try:
        value = cast(
            object,
            json.loads(
                _receipt_path(session_id, turn_id, data_path).read_text(
                    encoding="utf-8"
                )
            ),
        )
    except (OSError, json.JSONDecodeError):
        return None
    return _object_map(value)


def _receipt_authorizes_unavailable(receipt: Mapping[str, object]) -> bool:
    return (
        receipt.get("protocol") == PROTOCOL
        and receipt.get("browser_type") == "chrome"
        and receipt.get("status") == "unavailable"
        and receipt.get("can_report_unavailable") is True
        and receipt.get("probe_sha256") == EXPECTED_PROBE_SHA256
    )


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "print-probe-code":
        _ = sys.stdout.write(canonical_probe_code())
        return 0
    try:
        payload = cast(object, json.loads(sys.stdin.read()))
    except json.JSONDecodeError:
        return 0
    values = _object_map(payload)
    if values is None:
        return 0
    if values.get("hook_event_name") == "PostToolUse":
        _ = process_post_tool_use(values)
        return 0
    output = process_stop(values)
    if output is not None:
        print(json.dumps(output, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
