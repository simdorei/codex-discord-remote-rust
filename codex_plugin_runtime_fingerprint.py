from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Protocol, cast, override


REMOTE_PLUGIN_ID = "codex-discord-remote@codex-discord-remote"
CHROME_PLUGIN_ID = "chrome@openai-bundled"
REQUIRED_PLUGIN_IDS = (REMOTE_PLUGIN_ID, CHROME_PLUGIN_ID)
_IGNORED_DIRECTORY_NAMES = {"__pycache__"}
_IGNORED_SUFFIXES = {".pyc", ".pyo"}


class _HashWriter(Protocol):
    def update(self, data: bytes, /) -> None: ...


class PluginRuntimeFingerprintError(RuntimeError):
    @override
    def __str__(self) -> str:
        return self.args[0] if self.args else "plugin runtime fingerprint failed"


class PluginInventoryFingerprintError(PluginRuntimeFingerprintError):
    pass


class PluginContentFingerprintError(PluginRuntimeFingerprintError):
    pass


def read_codex_plugin_inventory() -> str:
    executable = os.environ.get("CODEX_EXE", "").strip() or shutil.which("codex")
    if executable is None:
        raise PluginRuntimeFingerprintError("Codex executable was not found on PATH")
    try:
        completed = subprocess.run(
            [executable, "plugin", "list", "--json"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=10.0,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        raise PluginRuntimeFingerprintError(
            f"Codex plugin inventory query failed: {exc}"
        ) from exc
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "no output"
        raise PluginRuntimeFingerprintError(
            "Codex plugin inventory query failed "
            + f"(exit {completed.returncode}): {detail}"
        )
    return completed.stdout


def capture_required_plugin_fingerprint() -> str:
    return fingerprint_required_plugins(read_codex_plugin_inventory())


def fingerprint_required_plugins(inventory_json: str) -> str:
    inventory = _inventory_object(inventory_json)
    records = _plugin_records(inventory.get("installed"))
    evidence = tuple(_plugin_evidence(records, plugin_id) for plugin_id in REQUIRED_PLUGIN_IDS)
    canonical = json.dumps(
        evidence,
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def _inventory_object(inventory_json: str) -> dict[str, object]:
    try:
        raw = cast(object, json.loads(inventory_json))
    except json.JSONDecodeError as exc:
        raise PluginInventoryFingerprintError(
            f"Codex plugin inventory is not valid JSON: {exc}"
        ) from exc
    if not isinstance(raw, dict):
        raise PluginInventoryFingerprintError(
            "Codex plugin inventory must be a JSON object"
        )
    return cast("dict[str, object]", raw)


def _plugin_records(value: object) -> tuple[dict[str, object], ...]:
    if not isinstance(value, list):
        raise PluginInventoryFingerprintError(
            "Codex plugin inventory.installed must be a JSON array"
        )
    records: list[dict[str, object]] = []
    for item in cast("list[object]", value):
        if not isinstance(item, dict):
            raise PluginInventoryFingerprintError(
                "Codex plugin inventory entries must be JSON objects"
            )
        records.append(cast("dict[str, object]", item))
    return tuple(records)


def _plugin_evidence(
    records: tuple[dict[str, object], ...], plugin_id: str
) -> dict[str, str]:
    matches = tuple(record for record in records if record.get("pluginId") == plugin_id)
    if len(matches) != 1:
        raise PluginInventoryFingerprintError(
            f"plugin {plugin_id!r} was not installed exactly once"
        )
    record = matches[0]
    if record.get("installed") is not True:
        raise PluginInventoryFingerprintError(
            f"plugin {plugin_id!r} is not installed"
        )
    if record.get("enabled") is not True:
        raise PluginInventoryFingerprintError(f"plugin {plugin_id!r} is not enabled")
    version = record.get("version")
    if not isinstance(version, str) or not version:
        raise PluginInventoryFingerprintError(
            f"plugin {plugin_id!r} version must be a non-empty string"
        )
    root = _plugin_source_path(record, plugin_id)
    return {
        "plugin_id": plugin_id,
        "version": version,
        "source_path": os.path.normcase(str(root)),
        "tree_sha256": _tree_digest(root, plugin_id),
    }


def _plugin_source_path(record: dict[str, object], plugin_id: str) -> Path:
    raw_source = record.get("source")
    if not isinstance(raw_source, dict):
        raise PluginInventoryFingerprintError(
            f"plugin {plugin_id!r} source must be a JSON object"
        )
    raw_path = cast("dict[str, object]", raw_source).get("path")
    if not isinstance(raw_path, str) or not raw_path:
        raise PluginInventoryFingerprintError(
            f"plugin {plugin_id!r} source.path must be a non-empty string"
        )
    try:
        root = Path(raw_path).resolve(strict=True)
    except OSError as exc:
        raise PluginContentFingerprintError(
            f"plugin {plugin_id!r} source path is unavailable: {exc}"
        ) from exc
    if not root.is_dir():
        raise PluginContentFingerprintError(
            f"plugin {plugin_id!r} source path is not a directory"
        )
    return root


def _tree_digest(root: Path, plugin_id: str) -> str:
    digest = hashlib.sha256()
    try:
        for current_raw, directory_names, file_names in os.walk(
            root,
            topdown=True,
            onerror=_raise_walk_error,
            followlinks=False,
        ):
            current = Path(current_raw)
            directory_names[:] = _safe_directories(
                root, current, directory_names, plugin_id
            )
            for file_name in sorted(file_names):
                path = current / file_name
                relative = path.relative_to(root)
                if _ignored_path(relative):
                    continue
                _reject_link_or_escape(root, path, relative, plugin_id)
                if path.is_file():
                    _hash_file(digest, relative.as_posix(), path)
    except OSError as exc:
        raise PluginContentFingerprintError(
            f"plugin {plugin_id!r} source tree could not be hashed: {exc}"
        ) from exc
    return digest.hexdigest()


def _raise_walk_error(error: OSError) -> None:
    raise error


def _safe_directories(
    root: Path,
    current: Path,
    directory_names: list[str],
    plugin_id: str,
) -> list[str]:
    safe: list[str] = []
    for name in sorted(directory_names):
        path = current / name
        relative = path.relative_to(root)
        if _ignored_path(relative):
            continue
        _reject_link_or_escape(root, path, relative, plugin_id)
        safe.append(name)
    return safe


def _reject_link_or_escape(
    root: Path, path: Path, relative: Path, plugin_id: str
) -> None:
    if path.is_symlink() or is_directory_junction(path):
        raise PluginContentFingerprintError(
            f"plugin {plugin_id!r} source contains a symbolic link or junction: "
            + str(relative)
        )
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(root):
        raise PluginContentFingerprintError(
            f"plugin {plugin_id!r} source escapes its root: {relative}"
        )


def is_directory_junction(path: Path) -> bool:
    return os.path.isjunction(path)


def _ignored_path(relative: Path) -> bool:
    return any(part in _IGNORED_DIRECTORY_NAMES for part in relative.parts) or (
        relative.suffix.casefold() in _IGNORED_SUFFIXES
    )


def _hash_file(digest: _HashWriter, relative: str, path: Path) -> None:
    relative_bytes = relative.encode("utf-8")
    digest.update(len(relative_bytes).to_bytes(8, "big"))
    digest.update(relative_bytes)
    size = path.stat().st_size
    digest.update(size.to_bytes(8, "big"))
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
