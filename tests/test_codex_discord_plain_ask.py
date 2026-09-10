from __future__ import annotations

import unittest
from dataclasses import dataclass

import codex_discord_plain_ask as plain_ask


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: str = "channel"


class PlainAskDirectRunError(RuntimeError):
    pass


class PlainAskInteractiveTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        state: str = "",
        resolved_thread_id: str | None = None,
        target_ref: str = "-",
        normalized_reply: str | None = None,
    ) -> tuple[plain_ask.PlainAskInteractiveDeps, list[tuple[str, str, str, str, str, list[str]]], list[tuple[str, str, str, str, str]]]:
        prompts: list[tuple[str, str, str, str, str, list[str]]] = []
        submitted: list[tuple[str, str, str, str, str]] = []

        async def send_interactive_prompt(
            channel: plain_ask.PlainAskChannel,
            target_thread_id: str,
            target_ref_arg: str,
            state_arg: str,
            prompt_text: str,
            options: list[str],
        ) -> None:
            prompts.append((str(channel), target_thread_id, target_ref_arg, state_arg, prompt_text, options))

        async def submit_interactive_reply(
            channel: plain_ask.PlainAskChannel,
            target_thread_id: str,
            target_ref_arg: str,
            state_arg: str,
            reply: str,
        ) -> None:
            submitted.append((str(channel), target_thread_id, target_ref_arg, state_arg, reply))

        deps = plain_ask.PlainAskInteractiveDeps(
            get_interactive_state_for_thread=lambda target: (state, resolved_thread_id, target_ref),
            normalize_interactive_text_reply=lambda state_arg, prompt: normalized_reply,
            send_interactive_prompt=send_interactive_prompt,
            submit_interactive_reply=submit_interactive_reply,
            state_input="waiting-input",
            state_approval="waiting-approval",
        )
        return deps, prompts, submitted

    async def test_returns_ask_target_when_not_interactive(self) -> None:
        deps, prompts, submitted = self.make_deps(resolved_thread_id="resolved-thread")

        result = await plain_ask.handle_interactive_plain_ask(
            FakeMessage(),
            "hello",
            None,
            deps=deps,
        )

        self.assertFalse(result.handled)
        self.assertEqual(result.ask_target_thread_id, "resolved-thread")
        self.assertEqual(prompts, [])
        self.assertEqual(submitted, [])

    async def test_pending_input_prompt_is_resent_when_reply_is_not_normalized(self) -> None:
        deps, prompts, submitted = self.make_deps(
            state="waiting-input",
            resolved_thread_id="thread-1",
            target_ref="project:1",
            normalized_reply=None,
        )

        result = await plain_ask.handle_interactive_plain_ask(
            FakeMessage(),
            "maybe",
            None,
            deps=deps,
        )

        self.assertTrue(result.handled)
        self.assertEqual(prompts, [("channel", "thread-1", "project:1", "waiting-input", "Pending input", [])])
        self.assertEqual(submitted, [])

    async def test_normalized_reply_is_submitted(self) -> None:
        deps, prompts, submitted = self.make_deps(
            state="waiting-approval",
            resolved_thread_id="thread-1",
            target_ref="project:1",
            normalized_reply="yes",
        )

        result = await plain_ask.handle_interactive_plain_ask(
            FakeMessage(),
            "approve",
            None,
            deps=deps,
        )

        self.assertTrue(result.handled)
        self.assertEqual(prompts, [])
        self.assertEqual(submitted, [("channel", "thread-1", "project:1", "waiting-approval", "yes")])


class PlainAskDirectFlowTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        has_recent: bool = False,
        busy: bool = False,
        claimed: bool = True,
        fail_run: bool = False,
    ) -> tuple[plain_ask.PlainAskDirectDeps[plain_ask.PlainAskMessage, None], list[str]]:
        events: list[str] = []

        async def has_recent_codex_app_user_prompt(target_thread_id: str | None, prompt: str) -> bool:
            events.append(f"dedupe:{target_thread_id}:{prompt}")
            return has_recent

        async def is_thread_runner_busy(target_thread_id: str | None) -> bool:
            events.append(f"busy:{target_thread_id}")
            return busy

        async def handle_busy_plain_ask(
            message: plain_ask.PlainAskMessage,
            prompt: str,
            target_thread_id: str | None,
        ) -> None:
            events.append(f"handle_busy:{message.channel}:{target_thread_id}:{prompt}")

        async def claim_direct_ask_target(target_thread_id: str | None) -> bool:
            events.append(f"claim:{target_thread_id}")
            return claimed

        async def release_direct_ask_target(target_thread_id: str | None) -> None:
            events.append(f"release:{target_thread_id}")

        async def run_prompt_flow(
            channel: plain_ask.PlainAskChannel,
            prompt: str,
            *,
            source_message: plain_ask.PlainAskMessage,
            target_thread_id: str | None,
        ) -> None:
            events.append(f"run:{channel}:{source_message.channel}:{target_thread_id}:{prompt}")
            if fail_run:
                raise PlainAskDirectRunError("run failed")

        async def send_chunks(
            channel: plain_ask.PlainAskChannel,
            text: str,
            *,
            context: str = "send_chunks",
        ) -> None:
            events.append(f"send:{channel}:{context}:{text}")

        deps = plain_ask.PlainAskDirectDeps(
            has_recent_codex_app_user_prompt=has_recent_codex_app_user_prompt,
            is_thread_runner_busy=is_thread_runner_busy,
            mark_recent_discord_origin_prompt=lambda target, prompt: events.append(f"mark:{target}:{prompt}"),
            handle_busy_plain_ask=handle_busy_plain_ask,
            claim_direct_ask_target=claim_direct_ask_target,
            release_direct_ask_target=release_direct_ask_target,
            run_prompt_flow=run_prompt_flow,
            send_chunks=send_chunks,
            format_log_text_len=len,
            log=lambda text: events.append(f"log:{text}"),
        )
        return deps, events

    async def test_duplicate_recent_app_prompt_is_acknowledged_without_running_prompt(self) -> None:
        deps, events = self.make_deps(has_recent=True)

        await plain_ask.handle_direct_plain_ask(
            FakeMessage(),
            "hello",
            "thread-1",
            deps=deps,
        )

        self.assertEqual(events[0], "dedupe:thread-1:hello")
        self.assertIn(
            "send:channel:send_chunks:Already in Codex app. Skipping duplicate Discord delivery for this mapped thread.",
            events,
        )
        self.assertFalse(any(event.startswith("run:") for event in events))

    async def test_busy_runner_marks_origin_and_delegates_to_busy_handler(self) -> None:
        deps, events = self.make_deps(busy=True)

        await plain_ask.handle_direct_plain_ask(
            FakeMessage(),
            "hello",
            "thread-1",
            deps=deps,
        )

        self.assertIn("mark:thread-1:hello", events)
        self.assertIn("handle_busy:channel:thread-1:hello", events)
        self.assertFalse(any(event.startswith("claim:") for event in events))

    async def test_claim_failure_delegates_to_busy_handler(self) -> None:
        deps, events = self.make_deps(claimed=False)

        await plain_ask.handle_direct_plain_ask(
            FakeMessage(),
            "hello",
            "thread-1",
            deps=deps,
        )

        self.assertIn("claim:thread-1", events)
        self.assertIn("mark:thread-1:hello", events)
        self.assertIn("handle_busy:channel:thread-1:hello", events)
        self.assertFalse(any(event.startswith("run:") for event in events))

    async def test_direct_prompt_runs_and_releases_claim(self) -> None:
        deps, events = self.make_deps()

        await plain_ask.handle_direct_plain_ask(
            FakeMessage(),
            "hello",
            "thread-1",
            deps=deps,
        )

        self.assertIn("mark:thread-1:hello", events)
        self.assertIn("run:channel:channel:thread-1:hello", events)
        self.assertEqual(events[-1], "release:thread-1")

    async def test_direct_prompt_releases_claim_when_run_fails(self) -> None:
        deps, events = self.make_deps(fail_run=True)

        with self.assertRaisesRegex(PlainAskDirectRunError, "run failed"):
            await plain_ask.handle_direct_plain_ask(
                FakeMessage(),
                "hello",
                "thread-1",
                deps=deps,
            )

        self.assertIn("run:channel:channel:thread-1:hello", events)
        self.assertEqual(events[-1], "release:thread-1")


if __name__ == "__main__":
    _ = unittest.main()
