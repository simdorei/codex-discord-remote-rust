from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LOCKED_REQUIREMENT = re.compile(r"^([A-Za-z0-9_.-]+)(?:\[[^]]+\])?==[^ \\]+")


class DesktopRequirementsLockTests(unittest.TestCase):
    def test_direct_requirements_remain_in_the_human_maintained_input(self) -> None:
        requirements = {
            line.strip()
            for line in (ROOT / "requirements.in").read_text(encoding="utf-8").splitlines()
            if line.strip() and not line.lstrip().startswith("#")
        }

        self.assertEqual(
            requirements,
            {
                "discord.py>=2.7,<3",
                "httpx2[http2,brotli,zstd]>=2,<3",
                "pydantic>=2.11,<3",
                "websockets>=16,<17",
            },
        )

    def test_runtime_lock_uses_exact_versions_and_hashes(self) -> None:
        text = (ROOT / "requirements.txt").read_text(encoding="utf-8")
        blocks = re.split(r"(?m)(?=^[A-Za-z0-9_.-]+(?:\[[^]]+\])?==)", text)
        requirement_blocks = [block for block in blocks if LOCKED_REQUIREMENT.match(block)]

        self.assertGreater(len(requirement_blocks), 4)
        for block in requirement_blocks:
            first_line = block.splitlines()[0]
            self.assertRegex(first_line, LOCKED_REQUIREMENT)
            self.assertIn("--hash=sha256:", block)

    def test_release_manifest_pins_bootstrap_artifacts(self) -> None:
        manifest = json.loads((ROOT / "runtime-release.json").read_text(encoding="utf-8"))

        self.assertEqual(manifest["python"]["version"], "3.12.1")
        self.assertEqual(manifest["pip"]["version"], "26.2.1")
        for artifact in (manifest["python"], manifest["get_pip"]):
            self.assertTrue(artifact["url"].startswith("https://"))
            self.assertRegex(artifact["sha256"], r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    _ = unittest.main()
