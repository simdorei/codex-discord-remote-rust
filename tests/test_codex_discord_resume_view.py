from __future__ import annotations

import unittest

import codex_discord_resume_view as resume_view


class ResumeButtonResponseTests(unittest.IsolatedAsyncioTestCase):
    async def test_recovery_uses_the_exact_failed_thread_and_does_not_replay_prompt(self) -> None:
        calls: list[tuple[int, str | None]] = []

        async def recover(channel_id: int, ref: str | None) -> str:
            calls.append((channel_id, ref))
            return "recovered; no prompt resent"

        deps = _build_deps(recover, [])

        response = await resume_view.build_resume_button_response(
            123,
            "thread-1",
            deps=deps,
        )

        self.assertEqual(calls, [(123, "thread-1")])
        self.assertEqual(response, "recovered; no prompt resent")

    async def test_recovery_surfaces_the_actual_error(self) -> None:
        logs: list[str] = []

        async def recover(channel_id: int, ref: str | None) -> str:
            _ = channel_id, ref
            raise TimeoutError("resume check timed out")

        response = await resume_view.build_resume_button_response(
            123,
            "thread-1",
            deps=_build_deps(recover, logs),
        )

        self.assertIn("ERROR: resume check timed out", response)
        self.assertIn("No prompt was resent", response)
        self.assertIn("resident_thread_resume_button_failed", logs[0])


def _build_deps(
    recover: resume_view.RecoverResidentThreadFunc,
    logs: list[str],
) -> resume_view.ResumeActionDeps:
    return resume_view.ResumeActionDeps(
        recover_resident_thread_for_request=recover,
        log=logs.append,
    )


if __name__ == "__main__":
    _ = unittest.main()
