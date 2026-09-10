from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from scripts.verify_plugin_cachebuster import (
    CachebusterError,
    MANIFEST_PATH,
    verify_plugin_cachebuster,
)


class PluginCachebusterTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.repo = Path(self.temporary_directory.name)
        self.manifest_path = self.repo / MANIFEST_PATH
        self.payload_path = self.repo / "plugins/codex-discord-remote/hooks/hook.py"

        self._git("init")
        self._git("config", "user.email", "cachebuster-test@example.invalid")
        self._git("config", "user.name", "Cachebuster Test")
        self._write_manifest(version="0.1.0+codex.20260829000000")
        self.payload_path.parent.mkdir(parents=True, exist_ok=True)
        self.payload_path.write_text("old\n", encoding="utf-8")
        self._commit("base")
        self.base_ref = self._git("rev-parse", "HEAD")

    def _git(self, *args: str) -> str:
        completed = subprocess.run(
            ["git", "-C", str(self.repo), *args],
            check=True,
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        return completed.stdout.strip()

    def _commit(self, message: str) -> None:
        self._git("add", ".")
        self._git("commit", "-m", message)

    def _write_manifest(
        self,
        *,
        version: str,
        description: str = "test plugin",
    ) -> None:
        self.manifest_path.parent.mkdir(parents=True, exist_ok=True)
        self.manifest_path.write_text(
            json.dumps(
                {
                    "name": "codex-discord-remote",
                    "version": version,
                    "description": description,
                }
            ),
            encoding="utf-8",
        )

    def test_packaged_change_without_version_bump_fails(self) -> None:
        self.payload_path.write_text("new\n", encoding="utf-8")
        self._commit("change payload")

        with self.assertRaisesRegex(CachebusterError, "without an increasing"):
            verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_packaged_change_with_version_bump_passes(self) -> None:
        self.payload_path.write_text("new\n", encoding="utf-8")
        self._write_manifest(version="0.1.0+codex.20260829000001")
        self._commit("change payload and version")

        verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_non_version_manifest_change_requires_version_bump(self) -> None:
        self._write_manifest(
            version="0.1.0+codex.20260829000000",
            description="changed description",
        )
        self._commit("change manifest payload")

        with self.assertRaises(CachebusterError):
            verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_packaged_change_rejects_version_downgrade(self) -> None:
        self.payload_path.write_text("new\n", encoding="utf-8")
        self._write_manifest(version="0.1.0+codex.20260828000000")
        self._commit("change payload and downgrade version")

        with self.assertRaisesRegex(CachebusterError, "without an increasing"):
            verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_packaged_change_rejects_invalid_cachebuster_date(self) -> None:
        self.payload_path.write_text("new\n", encoding="utf-8")
        self._write_manifest(version="0.1.0+codex.20261301000000")
        self._commit("change payload with invalid cachebuster")

        with self.assertRaisesRegex(CachebusterError, "invalid plugin cachebuster"):
            verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_version_only_change_does_not_require_payload_change(self) -> None:
        self._write_manifest(version="0.1.0+codex.20260829000001")
        self._commit("bump version only")

        verify_plugin_cachebuster(self.repo, self.base_ref)

    def test_version_only_downgrade_fails(self) -> None:
        self._write_manifest(version="0.1.0+codex.20260828000000")
        self._commit("downgrade version only")

        with self.assertRaisesRegex(CachebusterError, "without increasing"):
            verify_plugin_cachebuster(self.repo, self.base_ref)


if __name__ == "__main__":
    _ = unittest.main()
