import unittest
from dataclasses import dataclass

import discord

import codex_discord_button_qa_cases as button_qa_cases


@dataclass(frozen=True)
class FakeChannel:
    id: int = 222


@dataclass(frozen=True, slots=True)
class FakeAuthor:
    id: int = 333
    bot: bool = False


@dataclass(frozen=True, slots=True)
class FakeMessage:
    channel: discord.PartialMessageable
    author: FakeAuthor


@dataclass
class FakeButton:
    label: str | None
    custom_id: str | None


@dataclass
class FakeNonButton:
    label: str | None = "ignore me"
    custom_id: str | None = "not-a-button"


@dataclass(frozen=True)
class FakeView:
    children: tuple[button_qa_cases.ViewChildLike, ...]


class FakeSentMessage(str):
    @property
    def components(self) -> None:
        return None


class ButtonQaCaseTests(unittest.IsolatedAsyncioTestCase):
    def make_message(self) -> FakeMessage:
        client = discord.Client(intents=discord.Intents.none())
        self.addAsyncCleanup(client.close)
        return FakeMessage(
            channel=client.get_partial_messageable(222),
            author=FakeAuthor(),
        )

    def make_deps(
        self,
        *,
        children: list[button_qa_cases.ViewChildLike] | None = None,
        parsed_choice: tuple[str, str] | None = ("choice-1", "ignore"),
    ) -> tuple[button_qa_cases.BusyChoiceQaCaseDeps, list[tuple[str, object]]]:
        events: list[tuple[str, object]] = []
        view = FakeView(
            tuple(
                children
                or [
                    FakeButton("Ignore", "ignore-id"),
                    FakeButton("Queue next", "queue-id"),
                    FakeNonButton(),
                ]
            )
        )

        def get_mirrored_codex_thread_id(channel_id: int | None) -> str | None:
            events.append(("target", channel_id))
            return "thread-1"

        def make_busy_choice_payload(
            source_message: button_qa_cases.ButtonQaMessage,
            prompt: str,
            *,
            target_thread_id: str | None = None,
            allow_steer: bool = True,
        ) -> tuple[str, FakeView]:
            events.append(("payload", (source_message, prompt, target_thread_id, allow_steer)))
            return "content", view

        async def send_message_tracked(
            channel: object,
            content: str,
            *,
            view: object | None = None,
            context: str = "send_message_tracked",
        ) -> FakeSentMessage:
            events.append(("send", (channel, content, view, context)))
            return FakeSentMessage("sent-message")

        def parse_busy_choice_custom_id(custom_id: str) -> tuple[str, str] | None:
            events.append(("parse", custom_id))
            return parsed_choice

        deps = button_qa_cases.BusyChoiceQaCaseDeps(
            get_mirrored_codex_thread_id=get_mirrored_codex_thread_id,
            make_busy_choice_payload=make_busy_choice_payload,
            send_message_tracked=send_message_tracked,
            parse_busy_choice_custom_id=parse_busy_choice_custom_id,
            is_button=lambda item: isinstance(item, FakeButton),
        )
        return deps, events

    async def test_sends_busy_choice_case_and_collects_custom_ids(self) -> None:
        deps, events = self.make_deps()
        message = self.make_message()
        channel = FakeChannel()

        result = await button_qa_cases.send_busy_choice_qa_case(
            message,
            channel,
            "QA button ignore smoke",
            deps=deps,
        )

        self.assertEqual(result.sent_message, "sent-message")
        self.assertEqual(result.custom_ids, {"Ignore": "ignore-id", "Queue next": "queue-id"})
        self.assertEqual(result.choice_id, "choice-1")
        self.assertEqual(events[0], ("target", 222))
        self.assertEqual(events[1], ("payload", (message, "QA button ignore smoke", "thread-1", True)))
        self.assertEqual(events[2], ("send", (channel, "content", events[1][1] and result.view, "button_qa_busy_choice")))
        self.assertEqual(events[3], ("parse", "ignore-id"))

    async def test_preserves_missing_ignore_and_malformed_parse_edges(self) -> None:
        deps, events = self.make_deps(children=[FakeButton("Queue next", "queue-id")])
        message = self.make_message()

        with self.assertRaises(KeyError):
            _ = await button_qa_cases.send_busy_choice_qa_case(message, FakeChannel(), "prompt", deps=deps)

        self.assertFalse(any(event == ("parse", "queue-id") for event in events))

        deps, _events = self.make_deps(parsed_choice=None)
        result = await button_qa_cases.send_busy_choice_qa_case(message, FakeChannel(), "prompt", deps=deps)
        self.assertEqual(result.choice_id, "")


if __name__ == "__main__":
    _ = unittest.main()
