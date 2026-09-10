from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import codex_desktop_bridge_sidecar_resolver as resolver
import codex_app_server_transport_resident as resident


class CodexAppServerExecutableResolverTests(unittest.TestCase):
    def test_valid_codex_exe_remains_highest_priority(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            configured = self._touch(root / "configured" / self._executable_name())
            fallback = self._touch(root / "fallback" / self._executable_name())
            logs: list[str] = []

            with (
                mock.patch.object(resolver, "CODEX_APP_SERVER_EXE", str(configured)),
                mock.patch.object(
                    resolver,
                    "detect_running_codex_app_server_executable",
                    return_value=(fallback, "test-running"),
                ) as detect_running,
                mock.patch.object(
                    resolver,
                    "iter_codex_app_server_bin_candidates",
                    return_value=iter((("local-app-bin:test", fallback),)),
                ) as iter_local,
            ):
                selected = resolver.resolve_codex_app_server_executable(log_func=logs.append)

            self.assertEqual(selected, str(configured))
            detect_running.assert_not_called()
            iter_local.assert_not_called()
            self.assertEqual(
                logs,
                [f"codex_executable_selected source=env executable={configured}"],
            )

    def test_missing_codex_exe_does_not_block_existing_fallback_priority(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            stale = root / "deleted-hash" / self._executable_name()
            sandbox = self._touch(root / ".sandbox-bin" / self._executable_name())
            running = self._touch(root / "running" / self._executable_name())
            local = self._touch(root / "local" / self._executable_name())
            path_executable = self._touch(root / "path" / self._executable_name())
            logs: list[str] = []

            with (
                mock.patch.object(resolver, "CODEX_APP_SERVER_EXE", str(stale)),
                mock.patch.object(resolver, "CODEX_HOME", root),
                mock.patch.object(
                    resolver,
                    "detect_running_codex_app_server_executable",
                    return_value=(running, "test-running"),
                ) as detect_running,
                mock.patch.object(
                    resolver,
                    "iter_codex_app_server_bin_candidates",
                    return_value=iter((("local-app-bin:test", local),)),
                ) as iter_local,
                mock.patch.object(
                    resolver.shutil,
                    "which",
                    side_effect=lambda command: str(path_executable)
                    if command in ("codex", "codex.exe")
                    else None,
                ),
            ):
                selected = resolver.resolve_codex_app_server_executable(log_func=logs.append)

            self.assertEqual(selected, str(sandbox))
            detect_running.assert_not_called()
            iter_local.assert_not_called()
            self.assertEqual(
                logs,
                [
                    "codex_executable_config_skipped "
                    + f"source=env configured={str(stale)!r} reason=not_found_or_not_executable",
                    f"codex_executable_selected source=sandbox-bin executable={sandbox}",
                ],
            )

    def test_codex_exe_command_name_resolves_through_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            resolved = self._touch(Path(temp_dir) / self._executable_name())

            with (
                mock.patch.object(resolver, "CODEX_APP_SERVER_EXE", self._executable_name()),
                mock.patch.object(
                    resolver.shutil,
                    "which",
                    side_effect=lambda command: str(resolved)
                    if command == self._executable_name()
                    else None,
                ),
            ):
                selected = resolver.resolve_codex_app_server_executable()

            self.assertEqual(selected, str(resolved))

    def test_codex_exe_expands_environment_variables(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            configured = self._touch(root / self._executable_name())
            configured_value = f"$CODEX_RESOLVER_TEST_ROOT/{self._executable_name()}"

            with (
                mock.patch.dict(os.environ, {"CODEX_RESOLVER_TEST_ROOT": str(root)}),
                mock.patch.object(resolver, "CODEX_APP_SERVER_EXE", configured_value),
            ):
                selected = resolver.resolve_codex_app_server_executable()

            self.assertEqual(selected, str(configured))

    def test_resident_transport_default_resolver_forwards_log_callback(self) -> None:
        logs: list[str] = []

        def resolve_with_log(*, log_func: resolver.LogFunc | None = None) -> str:
            self.assertIsNotNone(log_func)
            if log_func is not None:
                log_func("codex_executable_selected source=test executable=selected.exe")
            return "selected.exe"

        with mock.patch.object(
            resident.bridge_resolver,
            "resolve_codex_app_server_executable",
            side_effect=resolve_with_log,
        ):
            transport = resident.ResidentCodexAppServerTransport(log_func=logs.append)
            selected = transport.executable_resolver()

        self.assertEqual(selected, "selected.exe")
        self.assertEqual(
            logs,
            ["codex_executable_selected source=test executable=selected.exe"],
        )

    @staticmethod
    def _executable_name() -> str:
        return "codex.exe" if os.name == "nt" else "codex"

    @staticmethod
    def _touch(path: Path) -> Path:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
        return path


if __name__ == "__main__":
    _ = unittest.main()
