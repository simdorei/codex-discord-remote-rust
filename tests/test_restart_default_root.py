"""Windows PowerShell -File must resolve the script folder after parameter binding."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class RestartDefaultRootTests(unittest.TestCase):
    def test_dry_run_without_repo_root_from_unrelated_directory(self):
        with tempfile.TemporaryDirectory(prefix='restart path ') as temp:
            folder = Path(temp)
            shutil.copyfile(ROOT / 'codex-discord-rust-restart.ps1', folder / 'restart.ps1')
            (folder / 'codex-discord-rust-watchdog.ps1').write_text(
                'param($RepoRoot,$BinaryPath,[switch]$DryRun,$RestartQuietSeconds)\n'
                'if(-not $DryRun){throw "not a dry run"}\n'
                'Write-Output $RepoRoot\nexit 0\n', encoding='utf-8')
            result = subprocess.run(
                ['powershell.exe', '-NoProfile', '-File', str(folder / 'restart.ps1'), '-DryRun'],
                cwd=folder.parent, capture_output=True, encoding='utf-8', errors='replace', timeout=15)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(result.stdout.strip(), str(folder))
