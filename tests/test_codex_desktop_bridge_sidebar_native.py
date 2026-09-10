from __future__ import annotations

import os
import subprocess
import sys
import unittest
from pathlib import Path


class SidebarNativeTests(unittest.TestCase):
    def test_stale_supplied_window_handle_is_rejected_before_ui_automation(self) -> None:
        if sys.platform != "win32":
            self.skipTest("Windows UI Automation script")
        script_path = Path(__file__).resolve().parents[1] / "codex_desktop_bridge_sidebar_native.ps1"
        env = os.environ.copy()
        env["CODEX_WINDOW_HANDLE"] = "2147483647"

        result = subprocess.run(
            [
                "powershell",
                "-NoProfile",
                "-Command",
                f". '{script_path}'; [long](Get-CodexWindowHandle)",
            ],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=20.0,
            check=False,
            env=env,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "0")


if __name__ == "__main__":
    unittest.main()
