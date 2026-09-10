from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from typing import Any


PLUGIN_ROOT = "plugins/codex-discord-remote"
MANIFEST_PATH = f"{PLUGIN_ROOT}/.codex-plugin/plugin.json"
VERSION_PATTERN = re.compile(
    r"^(?P<major>0|[1-9]\d*)\."
    r"(?P<minor>0|[1-9]\d*)\."
    r"(?P<patch>0|[1-9]\d*)\+codex\."
    r"(?P<cachebuster>\d{14})$"
)


class CachebusterError(RuntimeError):
    pass


def _git(repo: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    return completed.stdout.strip()


def _manifest_at(repo: Path, revision: str) -> dict[str, Any]:
    raw = _git(repo, "show", f"{revision}:{MANIFEST_PATH}")
    parsed = json.loads(raw)
    if not isinstance(parsed, dict) or not isinstance(parsed.get("version"), str):
        raise CachebusterError(f"invalid plugin manifest at {revision}")
    return parsed


def _without_version(manifest: dict[str, Any]) -> dict[str, Any]:
    return {key: value for key, value in manifest.items() if key != "version"}


def _version_key(version: str) -> tuple[int, int, int, int]:
    match = VERSION_PATTERN.fullmatch(version)
    if match is None:
        raise CachebusterError(f"invalid Codex plugin version: {version!r}")
    cachebuster = match.group("cachebuster")
    try:
        datetime.strptime(cachebuster, "%Y%m%d%H%M%S")
    except ValueError as exc:
        raise CachebusterError(f"invalid plugin cachebuster: {cachebuster}") from exc
    return (
        int(match.group("major")),
        int(match.group("minor")),
        int(match.group("patch")),
        int(cachebuster),
    )


def verify_plugin_cachebuster(repo: Path, base_ref: str) -> None:
    merge_base = _git(repo, "merge-base", "HEAD", base_ref)
    base_manifest = _manifest_at(repo, merge_base)
    head_manifest = _manifest_at(repo, "HEAD")
    base_version_key = _version_key(base_manifest["version"])
    head_version_key = _version_key(head_manifest["version"])
    changed_paths = set(
        _git(
            repo,
            "diff",
            "--name-only",
            merge_base,
            "HEAD",
            "--",
            PLUGIN_ROOT,
        ).splitlines()
    )

    packaged_file_changed = any(path != MANIFEST_PATH for path in changed_paths)
    manifest_payload_changed = _without_version(base_manifest) != _without_version(
        head_manifest
    )
    version_changed = head_manifest["version"] != base_manifest["version"]
    if not packaged_file_changed and not manifest_payload_changed:
        if version_changed and head_version_key <= base_version_key:
            raise CachebusterError(
                "plugin version changed without increasing its cache key: "
                f"update {MANIFEST_PATH}"
            )
        return

    if head_version_key <= base_version_key:
        raise CachebusterError(
            "packaged plugin content changed without an increasing plugin version: "
            f"update {MANIFEST_PATH}"
        )


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Require a new Codex plugin cachebuster when packaged content changes."
    )
    parser.add_argument(
        "--base-ref",
        default=os.environ.get("PLUGIN_VERSION_BASE_REF"),
        help="Git base ref; defaults to PLUGIN_VERSION_BASE_REF.",
    )
    args = parser.parse_args()
    if not args.base_ref:
        parser.error("--base-ref or PLUGIN_VERSION_BASE_REF is required")
    return args


def main() -> int:
    args = _parse_args()
    repo = Path(__file__).resolve().parents[1]
    try:
        verify_plugin_cachebuster(repo, args.base_ref)
    except (CachebusterError, json.JSONDecodeError, subprocess.CalledProcessError) as exc:
        print(f"plugin cachebuster verification failed: {exc}", file=sys.stderr)
        return 1
    print("plugin cachebuster verification passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
