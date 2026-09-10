"""Real PowerShell boundaries; only fixture process/start providers are substituted."""
from pathlib import Path
import os
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class RestartSafetyTests(unittest.TestCase):
    def run_ps(self, body, root):
        return subprocess.run(
            ['powershell.exe', '-NoProfile', '-Command', body],
            env={**os.environ, 'SAFETY_SOURCE': str(ROOT), 'SAFETY_ROOT': str(root)},
            capture_output=True, encoding='utf-8', errors='replace', timeout=20)

    def test_normal_exit_after_prepare_is_handed_off_not_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'watchdog.ps1').write_text('''
param($RepoRoot,$BinaryPath,$RestartQuietSeconds,$RestartWaitTimeoutSeconds,$CompleteRestartFenceJson)
if (-not $CompleteRestartFenceJson) { throw 'unbound general watchdog used' }
$f=$CompleteRestartFenceJson | ConvertFrom-Json
if($f.ProcessIdentity -ne '42|99' -or $f.Nonce -ne 'owned'){throw 'wrong authorization'}
[IO.File]::WriteAllText((Join-Path $RepoRoot 'completed'), 'yes')
exit 0
''', encoding='utf-8')
            result = self.run_ps(r'''
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $env:SAFETY_SOURCE 'codex-discord-rust-restart.ps1'),[ref]$tokens,[ref]$errors)
$ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst]},$false) | ForEach-Object {Invoke-Expression $_.Extent.Text}
$RepoRoot=$env:SAFETY_ROOT; $Watchdog=Join-Path $RepoRoot 'watchdog.ps1'
$BinaryPath='fixture'; $EffectiveDelaySeconds=0; $EffectiveQuietSeconds=15; $WaitTimeoutSeconds=0
$script:alive=$true
function Get-VerifiedRustIdentity {if($script:alive){'42|99'}else{''}}
function Invoke-RestartReadinessCheck {
    $script:alive=$false
    [pscustomobject]@{RuntimeId='old-runtime'; ProcessIdentity='42|99'; Nonce='owned'}
}
Invoke-BoundRustRestart -Identity '42|99'
''', root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((root / 'completed').exists())


if __name__ == '__main__':
    unittest.main()
