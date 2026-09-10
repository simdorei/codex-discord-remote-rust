"""Real update-only executable refusals, with no production marker writes."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(os.environ.get("CDR_TEST_OPERATOR_EXE"), "requires built update-only operator")
class UpdateOnlyOperatorTests(unittest.TestCase):
    def run_operator(self, arguments, cwd=ROOT):
        return subprocess.run(
            [os.environ["CDR_TEST_OPERATOR_EXE"], *arguments], cwd=cwd,
            capture_output=True, encoding="utf-8", errors="replace", timeout=20,
        )

    def test_unsupported_modes_and_extra_arguments_are_rejected_before_paths(self):
        for arguments in [[], ["cleanup"], ["delete", "--env", str(ROOT / ".env")],
                          ["cleanup", "--env", str(ROOT / ".env"), "--backup-store"]]:
            with self.subTest(arguments=arguments):
                result = self.run_operator(arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("maintenance_refused:", result.stderr)
                self.assertNotIn("confirmed", result.stdout)

    def test_foreign_root_and_env_are_rejected_without_reading_fixture(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture = root / "private.env"
            content = b"synthetic private sentinel must not be read or logged"
            fixture.write_bytes(content)
            for arguments, cwd, expected in [
                (["preflight", "--env", str(ROOT / ".env")], root, "maintenance root mismatch"),
                (["cleanup", "--env", str(fixture)], ROOT, "maintenance environment path mismatch"),
            ]:
                result = self.run_operator(arguments, cwd)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr)
                self.assertNotIn(content.decode(), result.stdout + result.stderr)
            self.assertEqual(fixture.read_bytes(), content)
            self.assertEqual([p.name for p in root.iterdir()], ["private.env"])


if __name__ == "__main__":
    unittest.main()
