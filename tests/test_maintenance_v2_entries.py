import json
import os
import subprocess
import shutil
import tempfile
import unittest
from test_rust_watchdog_restart_entry import RustWatchdogRestartEntryTests
from test_deployment_recovery import DeploymentRecoveryTests
from maintenance_v2_fixture import ROOT


class MaintenanceV2WatchdogEntryTests(unittest.TestCase):
    setUp = RustWatchdogRestartEntryTests.setUp
    run_entry = RustWatchdogRestartEntryTests.run_entry

    def test_actual_watchdog_stops_at_v2_owner_before_any_process_action(self):
        for stage in ['state_only', 'disabled_only', 'with_legacy_restart']:
            with self.subTest(stage=stage):
                marker = self.repo / '.codex_discord_rust.maintenance.v2'
                marker.write_text('owned v2 state', encoding='utf-8')
                if stage != 'with_legacy_restart':
                    for name in ['restart', 'drain.prepare', 'drain.ack']:
                        (self.repo / f'.codex_discord_rust.{name}').unlink(missing_ok=True)
                if stage == 'disabled_only':
                    (self.repo / '.codex_discord_bot.disabled').write_text('owned', encoding='utf-8')
                result = self.run_entry(old_alive=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('maintenance_v2_pending', result.stderr)
                self.assertNotIn('WAIT_EXIT', result.stdout)
                self.assertNotIn('START_ATTEMPT=', result.stdout)
                self.assertEqual(marker.read_text(), 'owned v2 state')


class MaintenanceV2RecoveryEntryTests(unittest.TestCase):
    setUp = DeploymentRecoveryTests.setUp
    recover = DeploymentRecoveryTests.recover

    def test_real_legacy_recovery_entry_rejects_v2_before_spawning_watchdog(self):
        state = json.loads(self.state.read_text())
        state['Version'] = 2
        self.state.write_text(json.dumps(state), encoding='utf-8')
        result = self.recover()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('maintenance_v2_or_unknown', result.stderr)
        self.assertFalse((self.root / 'started').exists())

    def test_real_legacy_deploy_entry_rejects_v2_without_stop_or_backup(self):
        state = json.loads(self.state.read_text())
        state['Version'] = 2
        self.state.write_text(json.dumps(state), encoding='utf-8')
        self.stop.unlink()
        result = subprocess.run(['powershell.exe', '-NoProfile', '-File',
            str(ROOT / 'scripts/Invoke-CdrDeployment.ps1'), '-StatePath', str(self.state)],
            capture_output=True, encoding='utf-8', errors='replace', timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('maintenance_v2_or_unknown', result.stderr)
        self.assertFalse(self.stop.exists())
        self.assertFalse((self.root / 'started').exists())


class MaintenanceV2OperationEntryTests(unittest.TestCase):
    def test_old_operation_cannot_adopt_new_active_state_or_completed_receipt(self):
        for completed in [False, True]:
            with self.subTest(completed=completed), tempfile.TemporaryDirectory() as temp:
                root = __import__('pathlib').Path(temp)
                (root/'scripts').mkdir()
                shutil.copyfile(ROOT/'scripts/Invoke-CdrMaintenance.ps1',root/'scripts/Invoke-CdrMaintenance.ps1')
                shutil.copyfile(ROOT/'codex-discord-rust-control.ps1',root/'codex-discord-rust-control.ps1')
                path = root/'.codex_discord_rust.maintenance.v2'
                record = path.with_name(path.name+'.completed') if completed else path
                payload = json.dumps(dict(Version=2,Operation='b'*32,Phase='verified' if completed else 'planned',Attempts=0))
                record.write_text(payload,encoding='utf-8')
                result = subprocess.run(['powershell.exe','-NoProfile','-File',str(root/'scripts/Invoke-CdrMaintenance.ps1'),
                    '-StatePath',str(path),'-ExpectedOperation','a'*32],capture_output=True,encoding='utf-8',errors='replace',timeout=15)
                self.assertNotEqual(result.returncode,0)
                self.assertIn('expected_operation_mismatch',result.stderr)
                self.assertEqual(record.read_text(),payload)
                self.assertFalse((root/'.codex_discord_rust.stop').exists())


if __name__ == '__main__':
    unittest.main()
