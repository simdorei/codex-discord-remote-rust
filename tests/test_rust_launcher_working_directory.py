from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.name == "nt", "Windows executable launcher contract")
class RustLauncherWorkingDirectoryTests(unittest.TestCase):
    def test_direct_launch_uses_repo_directory_and_preserves_exit_status(self) -> None:
        with tempfile.TemporaryDirectory(prefix="rust launcher ") as directory:
            base = Path(directory)
            repo = base / "repo with spaces"
            foreign = base / "unrelated working directory"
            repo.mkdir()
            foreign.mkdir()
            binary = repo / "target" / "release" / "cdr-runtime.exe"
            binary.parent.mkdir(parents=True)
            launcher = repo / "codex-discord-bot.cmd"
            shutil.copyfile(ROOT / launcher.name, launcher)
            (repo / ".env").write_text("# test only\n", encoding="utf-8")
            env = os.environ.copy()
            env["CODEX_DISCORD_RUNTIME"] = "rust"
            env["LAUNCHER_TEST_BINARY"] = str(binary)
            compile_result = subprocess.run(
                [
                    "powershell.exe", "-NoProfile", "-Command",
                    "Add-Type -OutputType ConsoleApplication "
                    "-OutputAssembly $env:LAUNCHER_TEST_BINARY -TypeDefinition "
                    "'using System; public class Probe { public static int Main(string[] args) "
                    "{ Console.WriteLine(\"CWD=\" + Environment.CurrentDirectory); "
                    "foreach (string arg in args) Console.WriteLine(\"ARG=\" + arg); "
                    "return 7; } }'",
                ],
                env=env, cwd=foreign, capture_output=True, text=True,
                errors="replace", timeout=30, check=False,
            )
            self.assertEqual(compile_result.returncode, 0, compile_result.stderr)
            result = subprocess.run(
                ["cmd.exe", "/c", str(launcher), "--probe"],
                env=env, cwd=foreign, capture_output=True, text=True,
                errors="replace", timeout=15, check=False,
            )
            output = result.stdout + result.stderr
            self.assertEqual(result.returncode, 7, output)
            self.assertIn(f"CWD={repo}", output)
            self.assertIn(f"ARG={repo / '.env'}", output)
            self.assertIn("ARG=--probe", output)
            self.assertFalse((foreign / "discord_mirror.sqlite").exists())

    def test_headless_sets_working_directory_before_launching(self) -> None:
        text = (ROOT / "codex-discord-bot-headless.vbs").read_text(encoding="utf-8-sig")
        self.assertLess(text.index("shell.CurrentDirectory = scriptDir"), text.index("shell.Run"))


if __name__ == "__main__":
    unittest.main()
