"""Execute real PowerShell boundaries with only fixture processes/files."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class RestartPreflightTests(unittest.TestCase):
    def run_ps(self, command, root):
        env = {**os.environ, 'RESTART_FIXTURE': str(root), 'RESTART_SOURCE': str(ROOT)}
        return subprocess.run(
            ['powershell.exe', '-NoProfile', '-Command', command], env=env,
            capture_output=True, encoding='utf-8', errors='replace', timeout=15)

    def test_failed_preflight_does_not_enter_drain(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'probe.cmd').write_text('@echo off\necho thread not loaded 1>&2\nexit /b 41\n')
            (root / 'watchdog.ps1').write_text(
                "[IO.File]::WriteAllText((Join-Path $env:RESTART_FIXTURE 'sealed'),'yes')\nexit 0\n")
            result = self.run_ps(r'''
                $ErrorActionPreference='Stop'
                $source=Get-Content (Join-Path $env:RESTART_SOURCE 'codex-discord-rust-restart.ps1') -Raw
                $tokens=$null; $errors=$null
                $ast=[Management.Automation.Language.Parser]::ParseInput($source,[ref]$tokens,[ref]$errors)
                if($errors.Count){throw 'parse error'}
                $ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$false) |
                    ForEach-Object { Invoke-Expression $_.Extent.Text }
                $RepoRoot=$env:RESTART_FIXTURE
                $BinaryPath=Join-Path $RepoRoot 'probe.cmd'
                $Watchdog=Join-Path $RepoRoot 'watchdog.ps1'
                $WaitTimeoutSeconds=0; $EffectiveQuietSeconds=15
                function Get-VerifiedRustIdentity { '42|99' }
                try { Invoke-RestartReadinessCheck -Identity '42|99'; throw 'preflight failure ignored' }
                catch { if($_.Exception.Message -notmatch 'preflight'){throw}; Write-Output $_.Exception.Message }
                if(Test-Path (Join-Path $RepoRoot 'sealed')) { throw 'intake was sealed despite failed preflight' }
            ''', root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertFalse((root / 'sealed').exists())
            self.assertIn('41', result.stdout)

    def test_queue_only_helper_refuses_before_any_stop(self):
        with tempfile.TemporaryDirectory() as temp:
            result = self.run_ps(r'''
                $ErrorActionPreference='Stop'
                $source=Get-Content (Join-Path $env:RESTART_SOURCE 'scripts/Restart-CdrAfterQueueIdle.ps1') -Raw
                $root=$env:RESTART_FIXTURE
                function Assert-OriginalRuntime { [pscustomobject]@{Id=42} }
                $CheckOnly=$false
                try { & ([scriptblock]::Create($source.Substring($source.IndexOf('Set-Location -LiteralPath $root')))); throw 'unsafe helper accepted' }
                catch { if($_.Exception.Message -notmatch 'Queue-only restart is retired'){throw} }
                if(Get-ChildItem -LiteralPath $root -Force){throw 'helper wrote a marker'}
                exit 0
            ''', Path(temp))
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
