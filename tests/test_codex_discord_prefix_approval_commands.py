import unittest
from dataclasses import dataclass

import codex_discord_prefix_approval_commands as prefix_approval


@dataclass(frozen=True, slots=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: FakeChannel

    @classmethod
    def make(cls, *, channel_id: int = 222) -> "FakeMessage":
        return cls(channel=FakeChannel(channel_id))


class PrefixApprovalCommandTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        mirrored_thread_id: str | None = "thread-1",
        selected_thread_id: str | None = "selected-thread",
        interactive_state: str = "waiting-approval",
        resolved_thread_id: str | None = "thread-1",
        target_ref: str = "project:1",
    ) -> tuple[
        prefix_approval.PrefixApprovalCommandDeps,
        list[str],
        list[str],
        list[tuple[prefix_approval.ChannelLike, str | None, str, str, str, list[str]]],
    ]:
        sent: list[str] = []
        events: list[str] = []
        prompts: list[tuple[prefix_approval.ChannelLike, str | None, str, str, str, list[str]]] = []

        async def send_chunks(target: prefix_approval.ChannelLike, text: str, *, context: str = "send_chunks") -> int:
            _ = target
            sent.append(f"{context}:{text}")
            return len(text)

        def get_mirrored_codex_thread_id(channel_id: int) -> str | None:
            events.append(f"mirror:{channel_id}")
            return mirrored_thread_id

        def resolve_selected_target() -> tuple[str | None, str]:
            events.append("selected")
            return selected_thread_id, selected_thread_id or "-"

        def get_interactive_state_for_thread(target_thread_id: str | None) -> tuple[str, str | None, str]:
            events.append(f"state:{target_thread_id}")
            return interactive_state, resolved_thread_id, target_ref

        def build_where_message(channel_id: int | None) -> str:
            events.append(f"where:{channel_id}")
            return f"where:{channel_id}"

        async def send_interactive_prompt(
            channel: prefix_approval.ChannelLike,
            target_thread_id: str | None,
            target_ref: str,
            state: str,
            prompt_text: str,
            choices: list[str],
        ) -> None:
            prompts.append((channel, target_thread_id, target_ref, state, prompt_text, choices))

        deps = prefix_approval.PrefixApprovalCommandDeps(
            send_chunks=send_chunks,
            get_mirrored_codex_thread_id=get_mirrored_codex_thread_id,
            resolve_selected_target=resolve_selected_target,
            get_interactive_state_for_thread=get_interactive_state_for_thread,
            build_where_message=build_where_message,
            send_interactive_prompt=send_interactive_prompt,
            interactive_state_approval="waiting-approval",
        )
        return deps, sent, events, prompts

    async def test_dispatches_pending_approval_prompt(self) -> None:
        deps, sent, events, prompts = self.make_deps()
        message = FakeMessage.make(channel_id=222)

        handled = await prefix_approval.handle_prefix_approval_command("approval", "", message, deps=deps)

        self.assertTrue(handled)
        self.assertEqual(sent, [])
        self.assertEqual(events, ["mirror:222", "state:thread-1"])
        self.assertEqual(prompts, [(message.channel, "thread-1", "project:1", "waiting-approval", "Pending approval", [])])

    async def test_preserves_no_target_no_pending_alias_and_unhandled(self) -> None:
        deps, sent, events, prompts = self.make_deps()
        message = FakeMessage.make()

        self.assertFalse(await prefix_approval.handle_prefix_approval_command("where", "", message, deps=deps))
        self.assertEqual(sent, [])
        self.assertEqual(events, [])
        self.assertEqual(prompts, [])

        deps, sent, events, prompts = self.make_deps(mirrored_thread_id=None, selected_thread_id=None)
        self.assertTrue(await prefix_approval.handle_prefix_approval_command("approval", "", message, deps=deps))
        self.assertEqual(events, ["mirror:222", "selected"])
        self.assertEqual(sent, ["prefix_approval_no_target:No Codex thread target found."])
        self.assertEqual(prompts, [])

        deps, sent, events, prompts = self.make_deps(
            interactive_state="waiting-input",
            resolved_thread_id="thread-1",
        )
        self.assertTrue(await prefix_approval.handle_prefix_approval_command("approval", "", message, deps=deps))
        self.assertEqual(events, ["mirror:222", "state:thread-1", "where:222"])
        self.assertEqual(sent, ["send_chunks:No pending approval for this Codex thread.\nwhere:222"])
        self.assertEqual(prompts, [])

        deps, sent, events, prompts = self.make_deps(
            mirrored_thread_id=None,
            selected_thread_id="selected-thread",
            resolved_thread_id="selected-thread",
            target_ref="selected:1",
        )
        self.assertTrue(await prefix_approval.handle_prefix_approval_command("approve", "", message, deps=deps))
        self.assertEqual(events, ["mirror:222", "selected", "state:selected-thread"])
        self.assertEqual(prompts, [(message.channel, "selected-thread", "selected:1", "waiting-approval", "Pending approval", [])])
        self.assertEqual(sent, [])


if __name__ == "__main__":
    _ = unittest.main()
