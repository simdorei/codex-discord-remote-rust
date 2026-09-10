"""Opt-in integration check against the built, real ticket operator; read-only refusals."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get('CDR_TEST_OPERATOR_EXE'), 'requires explicitly built operator')
class MaintenanceOperatorPinTests(unittest.TestCase):
    def test_override_is_rejected_before_database_or_rpc(self):
        operator = os.environ['CDR_TEST_OPERATOR_EXE']
        self.assertTrue(Path(operator).is_file())
        with tempfile.TemporaryDirectory() as temp:
            fixture = Path(temp) / 'fixture.sqlite'
            fixture.write_bytes(b'not a SQLite database; must never be queried')
            for key, path, error in [
                ('CODEX_STATE_DB', fixture, 'approved Codex home/state DB mismatch'),
                ('CODEX_HOME', Path(temp), 'approved Codex home/state DB mismatch'),
                ('CODEX_DISCORD_MIRROR_DB', fixture, 'maintenance DB mismatch'),
                ('CODEX_DISCORD_ROOT', Path(temp), 'legacy mutex root spelling mismatch'),
            ]:
                with self.subTest(key=key):
                    result = subprocess.run([operator, 'preflight', '--env', str(ROOT / '.env')],
                        cwd=ROOT, env={**os.environ, key: str(path)}, capture_output=True,
                        encoding='utf-8', errors='replace', timeout=20)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(error, result.stderr)
                    self.assertEqual(fixture.read_bytes(), b'not a SQLite database; must never be queried')
                    self.assertFalse(fixture.with_name(fixture.name+'-wal').exists())


if __name__ == '__main__':
    unittest.main()
