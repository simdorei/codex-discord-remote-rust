from __future__ import annotations

import hashlib
import json
import os
import re
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path
from typing import Final, TypeAlias, cast


JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]
JsonObject: TypeAlias = dict[str, JsonValue]


PROTOCOL: Final = "ask-chatgpt-pro-browser-evidence-v2"
EXPECTED_PROBE_SHA256: Final = (
    "346dfe09965e91f6b4339e6c1fcfb14a76c204ef3972779780847d2f7b358853"
)
PLUGIN_DATA_DIRECTORY: Final = "codex-discord-remote-codex-discord-remote"
PROBE_RELATIVE_PATH: Final = Path(
    "skills/ask-chatgpt-pro/scripts/browser_evidence_probe.mjs"
)


class ProChromeUnavailableError(RuntimeError):
    def __init__(self, internal_detail: str) -> None:
        super().__init__("pro_chrome_unavailable")
        self.internal_detail = internal_detail


def default_plugin_data_path() -> Path:
    return default_codex_home() / "plugins" / "data" / PLUGIN_DATA_DIRECTORY


def default_codex_home() -> Path:
    configured_home = os.environ.get("CODEX_HOME", "").strip()
    return Path(configured_home) if configured_home else Path.home() / ".codex"


def canonical_inner_probe_code(plugin_root: Path) -> str:
    probe_uri = (plugin_root / PROBE_RELATIVE_PATH).resolve().as_uri()
    encoded_uri = json.dumps(probe_uri, ensure_ascii=True)
    return (
        "nodeRepl.write(JSON.stringify(await (await import("
        f"{encoded_uri})).probeChrome(globalThis)));"
    )


def canonical_probe_code(plugin_root: Path) -> str:
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


def require_available_evidence(
    session_id: str,
    turn_id: str,
    *,
    plugin_data: Path | None = None,
    codex_home: Path | None = None,
    plugin_roots: Sequence[Path] | None = None,
) -> None:
    receipt = _read_receipt(
        session_id,
        turn_id,
        plugin_data or default_plugin_data_path(),
    )
    evidence = receipt
    if evidence is None:
        home = codex_home or default_codex_home()
        evidence = _read_transcript_evidence(
            session_id,
            turn_id,
            home,
            plugin_roots or _default_plugin_roots(home),
        )
    if evidence is None:
        raise ProChromeUnavailableError(
            "Pro turn completed without verified Chrome evidence "
            + "for the exact Codex session and turn."
        )
    status = evidence.get("status")
    if status != "available":
        raise ProChromeUnavailableError(
            "Pro turn did not acquire Chrome for the exact turn: "
            + f"status={status or 'missing'}"
        )
    if evidence.get("can_report_unavailable") is not False:
        raise ProChromeUnavailableError(
            "Verified Chrome evidence had an invalid availability flag."
        )


def _default_plugin_roots(codex_home: Path) -> tuple[Path, ...]:
    source_root = Path(__file__).resolve().parent / "plugins/codex-discord-remote"
    cache_root = (
        codex_home
        / "plugins/cache/codex-discord-remote/codex-discord-remote"
    )
    roots = [source_root]
    if cache_root.is_dir():
        roots.extend(path for path in cache_root.iterdir() if path.is_dir())
    return tuple(roots)


def _read_transcript_evidence(
    session_id: str,
    turn_id: str,
    codex_home: Path,
    plugin_roots: Sequence[Path],
) -> JsonObject | None:
    trusted_codes: set[str] = set()
    for root in plugin_roots:
        if not _probe_integrity_ok(root):
            continue
        code = canonical_probe_code(root)
        trusted_codes.update((code, f"{code}\n", f"{code}\r\n"))
    if not trusted_codes:
        return None

    latest_evidence: JsonObject | None = None
    for transcript in _transcript_paths(codex_home, session_id):
        trusted_calls: set[str] = set()
        try:
            lines = transcript.open(encoding="utf-8")
        except OSError:
            continue
        with lines:
            for line in lines:
                payload = _response_payload(line)
                if payload is None or _payload_turn_id(payload) != turn_id:
                    continue
                payload_type = payload.get("type")
                call_id = payload.get("call_id")
                if not isinstance(call_id, str) or not call_id:
                    continue
                if payload_type == "custom_tool_call":
                    if (
                        payload.get("name") in {"exec", "functions.exec"}
                        and payload.get("input") in trusted_codes
                    ):
                        trusted_calls.add(call_id)
                    continue
                if payload_type != "custom_tool_call_output" or call_id not in trusted_calls:
                    continue
                candidate = _find_valid_evidence(payload.get("output"))
                if candidate is not None:
                    latest_evidence = candidate
    return latest_evidence


def _transcript_paths(codex_home: Path, session_id: str) -> tuple[Path, ...]:
    if not session_id:
        return ()
    pattern = f"**/rollout-*-{session_id}.jsonl"
    paths: list[Path] = []
    for root_name in ("sessions", "archived_sessions"):
        root = codex_home / root_name
        if root.is_dir():
            paths.extend(root.glob(pattern))
    return tuple(sorted(paths, key=lambda path: path.stat().st_mtime))


def _response_payload(line: str) -> JsonObject | None:
    try:
        record = cast(JsonValue, json.loads(line))
    except json.JSONDecodeError:
        return None
    values = _object_map(record)
    if values is None or values.get("type") != "response_item":
        return None
    return _object_map(values.get("payload"))


def _payload_turn_id(payload: Mapping[str, JsonValue]) -> str | None:
    metadata = _object_map(payload.get("internal_chat_message_metadata_passthrough"))
    if metadata is None:
        return None
    turn_id = metadata.get("turn_id")
    return turn_id if isinstance(turn_id, str) else None


def _probe_integrity_ok(plugin_root: Path) -> bool:
    try:
        source = (plugin_root / PROBE_RELATIVE_PATH).read_bytes()
    except OSError:
        return False
    canonical_source = source.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    return hashlib.sha256(canonical_source).hexdigest() == EXPECTED_PROBE_SHA256


def _find_valid_evidence(raw: JsonValue) -> JsonObject | None:
    decoder = json.JSONDecoder()
    for text in _string_values(raw):
        for match in re.finditer(r"\{", text):
            try:
                value, _ = cast(
                    tuple[JsonValue, int], decoder.raw_decode(text[match.start() :])
                )
            except json.JSONDecodeError:
                continue
            values = _object_map(value)
            if values is not None and _valid_evidence(values):
                return values
    return None


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
    if not isinstance(raw, dict):
        return None
    return raw


def _valid_evidence(value: Mapping[str, JsonValue]) -> bool:
    status = value.get("status")
    can_report = value.get("can_report_unavailable")
    return (
        value.get("protocol") == PROTOCOL
        and value.get("browser_type") == "chrome"
        and status in {"available", "unavailable", "unverified"}
        and can_report is (status == "unavailable")
    )


def _read_receipt(
    session_id: str,
    turn_id: str,
    plugin_data: Path,
) -> JsonObject | None:
    if not session_id or not turn_id:
        return None
    key = hashlib.sha256(f"{session_id}\0{turn_id}".encode()).hexdigest()
    path = plugin_data / "browser-evidence" / f"{key}.json"
    try:
        raw = cast(JsonValue, json.loads(path.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError):
        return None
    receipt = _object_map(raw)
    if receipt is None:
        return None
    if (
        receipt.get("protocol") != PROTOCOL
        or receipt.get("session_id") != session_id
        or receipt.get("turn_id") != turn_id
        or receipt.get("browser_type") != "chrome"
        or receipt.get("probe_sha256") != EXPECTED_PROBE_SHA256
        or not isinstance(receipt.get("can_report_unavailable"), bool)
    ):
        return None
    return receipt
