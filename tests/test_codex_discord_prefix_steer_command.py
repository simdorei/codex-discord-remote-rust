import unittest
from collections.abc import AsyncGenerator
from contextlib import asynccontextmanager
from dataclasses import dataclass

import codex_discord_prefix_steer_command as prefix_steer
from codex_discord_steering import SteeringPromptResult


@dataclass(frozen=True)
class FakeAuthor:
    id: int = 333


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True)
class FakeMessage:
    channel: FakeChannel
    author: FakeAuthor

    @classmethod
    def make(cls, channel_id: int = 222, author_id: int = 333) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id), author=FakeAuthor(author_id))


@dataclass
class Records:
    sent: list[str]
    events: list[str]
    logs: list[str]
    streamed: list[tuple[SteeringPromptResult, str | None, dict[str, object | None]]]
    handoffs: list[str | None]


class PrefixSteerCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        qa_enabled: bool = True,
        mirrored_thread_id: str | None = None,
        selected_thread_id: str | None = "thread-1",
        mapped_result: bool = True,
        delegation_result: bool = False,
        steering_result: SteeringPromptResult | None = None,
    ) -> tuple[prefix_steer.PrefixSteerCommandDeps, Records]:
        records = Records(sent=[], events=[], logs=[], streamed=[], handoffs=[])
        result = steering_result or SteeringPromptResult(
            0,
            "[qa_delivery_verified]",
            target_thread_id=selected_thread_id,
            target_ref=selected_thread_id or "-",
            session_path="qa-session.jsonl",
            start_offset=0,
        )
        monotonic_values = iter([100.0, 101.25])

        async def send_chunks(target: object, text: str, *, context: str = "send_chunks") -> object:
            _ = target
            records.sent.append(f"{context}:{text}")
            return len(text)

        def get_mirrored_codex_thread_id(channel_id: int) -> str | None:
            records.events.append(f"mirror:{channel_id}")
            return mirrored_thread_id

        def resolve_selected_target() -> tuple[str | None, str]:
            records.events.append("selected")
            return selected_thread_id, selected_thread_id or "-"

        async def prepare_mapped_session_mirror_output(channel: object, target_thread_id: str | None) -> bool:
            _ = channel
            records.events.append(f"mapped:{target_thread_id}")
            return mapped_result

        async def prepare_session_mirror_delegation(channel: object, target_thread_id: str | None) -> bool:
            _ = channel
            records.events.append(f"delegation:{target_thread_id}")
            return delegation_result

        @asynccontextmanager
        async def channel_typing(channel: object, *, context: str = "typing") -> AsyncGenerator[None]:
            _ = channel
            records.events.append(f"typing:{context}")
            yield

        def run_steering_prompt(prompt: str, target_thread_id: str | None) -> SteeringPromptResult:
            records.events.append(f"run:{prompt}:{target_thread_id}")
            return result

        async def stream_steering_prompt_result_to_channel(
            channel: object,
            steering_result: SteeringPromptResult,
            target_thread_id: str | None,
            *,
            send_commentary_blocks: bool | None = None,
            send_final_blocks: bool = True,
        ) -> bool:
            _ = channel
            records.events.append(f"stream:{target_thread_id}")
            records.streamed.append(
                (
                    steering_result,
                    target_thread_id,
                    {
                        "send_commentary_blocks": send_commentary_blocks,
                        "send_final_blocks": send_final_blocks,
                    },
                )
            )
            return True

        def monotonic() -> float:
            return next(monotonic_values)

        deps = prefix_steer.PrefixSteerCommandDeps(
            send_chunks=send_chunks,
            qa_commands_enabled=lambda: qa_enabled,
            get_mirrored_codex_thread_id=get_mirrored_codex_thread_id,
            resolve_selected_target=resolve_selected_target,
            prepare_mapped_session_mirror_output=prepare_mapped_session_mirror_output,
            prepare_session_mirror_delegation=prepare_session_mirror_delegation,
            channel_typing=channel_typing,
            run_steering_prompt=run_steering_prompt,
            mark_steering_handoff=records.handoffs.append,
            stream_steering_prompt_result_to_channel=stream_steering_prompt_result_to_channel,
            log_line=records.logs.append,
            format_log_text_len=lambda text: str(len(text)),
            monotonic=monotonic,
        )
        return deps, records

    async def test_dispatches_enabled_steer_to_selected_target(self) -> None:
        deps, records = self.make_deps(
            mirrored_thread_id=None,
            selected_thread_id="thread-1",
            mapped_result=True,
        )
        message = FakeMessage.make(channel_id=222, author_id=333)

        handled = await prefix_steer.handle_prefix_steer_command("steer", "please steer now", message, deps=deps)

        self.assertTrue(handled)
        self.assertEqual(
            records.events,
            [
                "mirror:222",
                "selected",
                "mapped:thread-1",
                "typing:prefix_steer",
                "run:please steer now:thread-1",
                "stream:thread-1",
            ],
        )
        self.assertEqual(records.handoffs, ["thread-1"])
        self.assertEqual(
            records.sent,
            ["send_chunks:Steering sent\n\n[qa_delivery_verified]"],
        )
        self.assertEqual(len(records.streamed), 1)
        self.assertEqual(records.streamed[0][1], "thread-1")
        self.assertEqual(records.streamed[0][2], {"send_commentary_blocks": False, "send_final_blocks": False})
        self.assertTrue(any("prefix_steer channel=222 user=333 target=thread-1 prompt_len=16" in line for line in records.logs))
        self.assertTrue(any("prefix_steer_done exit=0 target=thread-1 elapsed_sec=1.25 output_len=22" in line for line in records.logs))
        self.assertTrue(any("prefix_steer_delegated_to_session_mirror target=thread-1" == line for line in records.logs))

    async def test_preserves_disabled_usage_no_target_failure_and_unhandled(self) -> None:
        deps, records = self.make_deps()
        message = FakeMessage.make()

        self.assertFalse(await prefix_steer.handle_prefix_steer_command("chatid", "", message, deps=deps))
        self.assertEqual(records.sent, [])

        deps, records = self.make_deps(qa_enabled=False)
        self.assertTrue(await prefix_steer.handle_prefix_steer_command("steer", "please", message, deps=deps))
        self.assertEqual(
            records.sent,
            ["prefix_steer_disabled:Discord QA steering is disabled. Set DISCORD_ENABLE_QA_COMMANDS=1 to enable it."],
        )
        self.assertEqual(records.events, [])

        deps, records = self.make_deps()
        self.assertTrue(await prefix_steer.handle_prefix_steer_command("steer", "", message, deps=deps))
        self.assertEqual(records.sent, ["prefix_steer_usage:Usage: !steer <prompt>"])
        self.assertEqual(records.events, [])

        deps, records = self.make_deps(mirrored_thread_id=None, selected_thread_id=None)
        self.assertTrue(await prefix_steer.handle_prefix_steer_command("steer", "please", message, deps=deps))
        self.assertEqual(records.sent, ["prefix_steer_no_target:No Codex thread target found."])
        self.assertEqual(records.events, ["mirror:222", "selected"])

        deps, records = self.make_deps(
            mirrored_thread_id="thread-2",
            mapped_result=False,
            delegation_result=False,
            steering_result=SteeringPromptResult(7, "failed"),
        )
        self.assertTrue(await prefix_steer.handle_prefix_steer_command("steer", "please", message, deps=deps))
        self.assertEqual(
            records.events,
            ["mirror:222", "mapped:thread-2", "delegation:thread-2", "typing:prefix_steer", "run:please:thread-2"],
        )
        self.assertEqual(records.handoffs, [])
        self.assertEqual(records.streamed, [])
        self.assertEqual(records.sent, ["send_chunks:Steering failed (exit 7)\n\nfailed"])


if __name__ == "__main__":
    _ = unittest.main()
