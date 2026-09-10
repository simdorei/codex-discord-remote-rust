from __future__ import annotations

import unittest

import codex_discord_origin_prompts as origin_prompts


class OriginPromptTests(unittest.TestCase):
    def test_cleanup_removes_expired_prompts_only(self) -> None:
        prompts = {"old": 1.0, "fresh": 9.0}

        origin_prompts.cleanup_recent_discord_origin_prompts(
            prompts,
            ttl_seconds=5.0,
            now=10.0,
        )

        self.assertEqual(prompts, {"fresh": 9.0})

    def test_mark_recent_prompt_records_digest_after_cleanup(self) -> None:
        prompts = {"old": 1.0}

        origin_prompts.mark_recent_discord_origin_prompt(
            prompts,
            "thread-1",
            " hello ",
            ttl_seconds=5.0,
            now=10.0,
        )

        self.assertEqual(len(prompts), 1)
        self.assertIn(origin_prompts.make_discord_origin_prompt_digest("thread-1", " hello "), prompts)

    def test_should_skip_pops_matching_prompt_once(self) -> None:
        digest = origin_prompts.make_discord_origin_prompt_digest("thread-1", "hello")
        prompts = {digest: 10.0}

        self.assertTrue(
            origin_prompts.should_skip_discord_origin_prompt(
                prompts,
                "thread-1",
                "hello",
                ttl_seconds=5.0,
                now=11.0,
            )
        )
        self.assertFalse(
            origin_prompts.should_skip_discord_origin_prompt(
                prompts,
                "thread-1",
                "hello",
                ttl_seconds=5.0,
                now=11.0,
            )
        )


if __name__ == "__main__":
    _ = unittest.main()
