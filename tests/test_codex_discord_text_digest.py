from __future__ import annotations

import hashlib
import unittest

import codex_discord_text_digest as text_digest


class TextDigestTests(unittest.TestCase):
    def test_make_text_digest_matches_existing_sha256_contract(self) -> None:
        expected = hashlib.sha256()
        expected.update(b"alpha")
        expected.update(b"\0")
        expected.update(b"beta")
        expected.update(b"\0")

        self.assertEqual(text_digest.make_text_digest("alpha", "beta"), expected.hexdigest())

    def test_make_text_digest_keeps_part_boundaries_distinct(self) -> None:
        self.assertNotEqual(
            text_digest.make_text_digest("a", "bc"),
            text_digest.make_text_digest("ab", "c"),
        )

    def test_make_text_digest_preserves_existing_falsey_part_behavior(self) -> None:
        self.assertEqual(
            text_digest.make_text_digest(0, False, None, ""),
            text_digest.make_text_digest("", "", "", ""),
        )
