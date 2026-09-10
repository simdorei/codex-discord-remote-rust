from __future__ import annotations

import unittest

import codex_discord_user_root_scope as user_root_scope
from codex_thread_models import ThreadInfo


class UserRootScopeTests(unittest.TestCase):
    def test_scope_keeps_all_state_roots(self) -> None:
        roots = [
            *[_thread(f"ordinary-{index}") for index in range(21)],
            _thread("root-22"),
            _thread("root-23"),
            _thread("root-24"),
        ]

        scoped = user_root_scope.load_ordinary_user_root_threads(
            lambda _limit: roots,
        )

        self.assertEqual(len(roots), 24)
        self.assertEqual(len(scoped), 24)
        self.assertEqual(scoped, roots)

    def test_limit_is_applied_after_loading_the_full_state_root_scope(self) -> None:
        observed_limits: list[int] = []

        def load_roots(limit: int) -> list[ThreadInfo]:
            observed_limits.append(limit)
            return [_thread("root-1"), _thread("root-2"), _thread("root-3")]

        scoped = user_root_scope.load_ordinary_user_root_threads(
            load_roots,
            limit=1,
        )

        self.assertEqual(observed_limits, [0])
        self.assertEqual([thread.id for thread in scoped], ["root-1"])


def _thread(thread_id: str) -> ThreadInfo:
    return ThreadInfo(
        id=thread_id,
        title=thread_id,
        cwd=r"C:\repo",
        updated_at=1,
        rollout_path=f"{thread_id}.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=0,
    )


if __name__ == "__main__":
    _ = unittest.main()
