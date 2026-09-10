"""Keep the Rust communication path free of the legacy Desktop IPC adapter.

This is a source boundary guard, not a live protocol test. Anonymous standard-I/O
pipes, Windows mutexes, and retained bridge-state JSON filenames are legitimate.
"""

from __future__ import annotations

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class AppServerOnlyBoundaryTests(unittest.TestCase):
    def test_live_bootstrap_installs_dead_generation_fence_before_sharing_server(self) -> None:
        source = (ROOT / "crates/cdr-runtime/src/discord_runtime.rs").read_text(encoding="utf-8")
        compact = "".join(source.split())
        self.assertIn(
            "start_app_server(&paths,drain_controller.runtime_id(),config.startup_channel_id,)",
            compact,
        )
        self.assertIn("RuntimeDeadGenerationFence::new(", source)
        self.assertIn("ResidentAppServer::start_with_dead_generation_fence(", source)
        self.assertNotIn("ResidentAppServer::start(config)", source)
        self.assertLess(source.index("RuntimeInstanceGuard::acquire"), source.index("let server ="))
        self.assertLess(source.index("prepare_storage(&paths)"), source.index("let server ="))
        self.assertLess(source.index("let server ="), source.index("build_executor("))

    def test_rust_production_modules_never_invoke_legacy_desktop_ipc(self) -> None:
        forbidden = (
            "ask_ipc", "pending_ipc", "pending_codex_desktop_request",
            "codex_desktop_bridge.py", "codex_bridge_ipc",
            "codex-discord-watchdog-restart-runtime.ps1",
        )
        scanned = 0
        for crate in ("cdr-runtime", "cdr-app-server", "cdr-discord", "cdr-core"):
            for path in (ROOT / "crates" / crate / "src").rglob("*.rs"):
                if path.stem.endswith("_tests") or "tests" in path.parts:
                    continue
                scanned += 1
                text = path.read_text(encoding="utf-8").casefold()
                for token in forbidden:
                    with self.subTest(path=str(path.relative_to(ROOT)), token=token):
                        self.assertNotIn(token, text)
        self.assertGreater(scanned, 50, "guard must inspect actual production source")


if __name__ == "__main__":
    unittest.main()
