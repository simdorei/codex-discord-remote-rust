"""Real candidate snapshot with synthetic data, no production credentials or DB."""
import hashlib
from contextlib import closing
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from maintenance_v2_fixture import ROOT, LOAD


class MaintenanceSnapshotIntegrationTests(unittest.TestCase):
    def test_candidate_backup_preserves_live_source_and_checks_snapshot(self):
        candidate = Path(os.environ['CDR_TEST_RUNTIME_EXE']).resolve(strict=True)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'discord_mirror.sqlite'
            with closing(sqlite3.connect(source)) as db, db:
                db.execute('CREATE TABLE synthetic_ticket(id INTEGER PRIMARY KEY, value TEXT)')
                db.execute("INSERT INTO synthetic_ticket VALUES (1, 'retain me')")
                db.execute('PRAGMA user_version=3')
            before = hashlib.sha256(source.read_bytes()).hexdigest()
            config = root / 'fixture.env'
            config.write_text('DISCORD_BOT_TOKEN=synthetic-not-a-real-token\nDISCORD_ALLOW_ALL_CHANNELS=1\n', encoding='utf-8')
            fake_codex = root / 'codex.exe'
            fake_codex.write_bytes(b'not executable; backup must not launch an app-server')
            env = {key: value for key, value in os.environ.items()
                   if key.upper() in {'PATH', 'SYSTEMROOT', 'WINDIR', 'TEMP', 'TMP', 'COMSPEC', 'PATHEXT'}}
            env.update(USERPROFILE=temp, CODEX_HOME=str(root/'codex-home'),
                       CODEX_EXE=str(fake_codex), CODEX_DISCORD_ROOT=temp,
                       CODEX_DISCORD_MIRROR_DB=str(source), CODEX_STATE_DB=str(root/'unused.sqlite'))
            result = subprocess.run([str(candidate), '--backup-store', '--env', str(config)],
                                    cwd=root, env=env, capture_output=True, encoding='utf-8', timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            lines = result.stdout.strip().splitlines()
            self.assertEqual(len(lines), 1, result.stdout)
            self.assertTrue(lines[0].startswith('backup_created path='))
            backup = Path(lines[0].removeprefix('backup_created path='))
            self.assertEqual(backup.parent, root/'.codex-discord-backups')
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(), before)
            with closing(sqlite3.connect(f'{backup.as_uri()}?mode=ro', uri=True)) as db:
                self.assertEqual(db.execute('PRAGMA integrity_check').fetchall(), [('ok',)])
                self.assertEqual(db.execute('PRAGMA user_version').fetchone(), (3,))
                self.assertEqual(db.execute('SELECT * FROM synthetic_ticket').fetchall(), [(1, 'retain me')])
            self.assertFalse((root/'unused.sqlite').exists())
            self.assertFalse((root/'.codex_discord_rust.stop').exists())
            self.assertFalse((root/'.codex_discord_rust.drain.identity').exists())
            # Exercise the real PowerShell backup/result/receipt wiring against the same candidate.
            env.update(V2_ROOT=temp,V2_SOURCE=str(ROOT),CDR_TEST_RUNTIME_EXE=str(candidate))
            script=LOAD+r'''
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceCommand.ps1')
. (Join-Path $env:V2_SOURCE 'scripts/CdrMaintenanceBackup.ps1')
$EnvPath=Join-Path $RepoRoot 'fixture.env'
$s=Read-CdrMaintenanceState $StatePath
[IO.File]::WriteAllText($BinaryPath,'MZsynthetic baseline')
[IO.File]::Copy($env:CDR_TEST_RUNTIME_EXE,$s.CandidatePath,$false)
[IO.File]::WriteAllText($s.OperatorPath,'MZsynthetic operator; never executed')
$s.BaselineHash=Get-CdrArtifactHash $BinaryPath;$s.CandidateHash=Get-CdrArtifactHash $s.CandidatePath
Save-CdrMaintenanceState $s $StatePath
Invoke-CdrMaintenancePreStopBackup $s $StatePath
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenancePreStopBackup $saved
New-CdrMaintenanceSnapshotReceipt $saved $StatePath 'PostStopBackup'
$saved=Read-CdrMaintenanceState $StatePath
Assert-CdrMaintenanceBackupReceipt $saved 'PostStopBackup'
if($saved.PreStopBackup.SnapshotPath -ceq $saved.PostStopBackup.SnapshotPath){throw 'same backup reused'}
if($null -ne $saved.ActiveCommand){throw 'native receipt not committed'}
Write-Output 'REAL_BACKUP_RECEIPTS_OK'
'''
            wrapped=subprocess.run(['powershell.exe','-NoProfile','-Command',script],cwd=root,env=env,
                                   capture_output=True,encoding='utf-8',errors='replace',timeout=25)
            self.assertEqual(wrapped.returncode,0,wrapped.stdout+wrapped.stderr)
            self.assertIn('REAL_BACKUP_RECEIPTS_OK',wrapped.stdout)
            self.assertEqual(hashlib.sha256(source.read_bytes()).hexdigest(),before)
            self.assertFalse((root/'.codex_discord_rust.stop').exists())


if __name__ == '__main__':
    unittest.main()
