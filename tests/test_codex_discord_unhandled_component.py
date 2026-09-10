from __future__ import annotations

import unittest
from dataclasses import dataclass, field

import codex_discord_unhandled_component as unhandled_component


@dataclass(frozen=True, slots=True)
class FakeResponse:
    done: bool = False

    def is_done(self) -> bool:
        return self.done


@dataclass(frozen=True, slots=True)
class FakeUser:
    id: int = 42


@dataclass(frozen=True, slots=True)
class FakeInteraction:
    custom_id: str = "button-1"
    channel_id: int = 123
    user: FakeUser = field(default_factory=FakeUser)
    response: FakeResponse = field(default_factory=FakeResponse)


class AlreadyAckError(RuntimeError):
    pass


class UnhandledComponentTests(unittest.IsolatedAsyncioTestCase):
    def make_deps(
        self,
        *,
        handled_by_persistent: bool = False,
        persistent_error: BaseException | None = None,
        report_error: BaseException | None = None,
    ) -> tuple[unhandled_component.UnhandledComponentDeps[FakeInteraction], list[str]]:
        events: list[str] = []

        async def sleep(delay: float) -> None:
            events.append(f"sleep:{delay}")

        async def handle_persistent(interaction: FakeInteraction, custom_id: str) -> bool:
            events.append(f"persistent:{custom_id}")
            if persistent_error is not None:
                raise persistent_error
            return handled_by_persistent

        async def clear_components(interaction: FakeInteraction, *, context: str) -> None:
            events.append(f"clear:{interaction.custom_id}:{context}")

        async def send_response(
            interaction: FakeInteraction,
            content: str,
            *,
            ephemeral: bool,
            context: str,
        ) -> None:
            events.append(f"send:{interaction.custom_id}:{ephemeral}:{context}:{content}")
            if report_error is not None:
                raise report_error

        deps = unhandled_component.UnhandledComponentDeps(
            sleep=sleep,
            get_custom_id=lambda interaction: interaction.custom_id,
            persistent_handlers=(handle_persistent,),
            clear_components=clear_components,
            send_response=send_response,
            is_already_acknowledged=lambda exc: isinstance(exc, AlreadyAckError),
            format_exception=lambda: "traceback text",
            delivery_exceptions=(AlreadyAckError, RuntimeError),
            log=lambda text: events.append(f"log:{text}"),
        )
        return deps, events

    async def test_done_response_is_ignored_after_delay(self) -> None:
        deps, events = self.make_deps()
        interaction = FakeInteraction(response=FakeResponse(done=True))

        await unhandled_component.report_unhandled_component_interaction(
            interaction,
            delay_sec=0.1,
            deps=deps,
        )

        self.assertEqual(events, ["sleep:0.1"])

    async def test_persistent_handler_claims_interaction(self) -> None:
        deps, events = self.make_deps(handled_by_persistent=True)

        await unhandled_component.report_unhandled_component_interaction(
            FakeInteraction(),
            delay_sec=0,
            deps=deps,
        )

        self.assertEqual(events, ["sleep:0", "persistent:button-1"])

    async def test_unhandled_button_clears_components_and_sends_stale_notice(self) -> None:
        deps, events = self.make_deps()

        await unhandled_component.report_unhandled_component_interaction(
            FakeInteraction(),
            delay_sec=0,
            deps=deps,
        )

        self.assertIn("clear:button-1:unhandled_component", events)
        self.assertTrue(
            any(event.startswith("send:button-1:True:component_unhandled:This Discord button") for event in events)
        )
        self.assertIn(
            "log:component_interaction_unhandled_reported custom_id=button-1 channel=123 user=42",
            events,
        )

    async def test_already_acknowledged_persistent_error_is_logged_without_report(self) -> None:
        deps, events = self.make_deps(persistent_error=AlreadyAckError("ack"))

        await unhandled_component.report_unhandled_component_interaction(
            FakeInteraction(),
            delay_sec=0,
            deps=deps,
        )

        self.assertIn(
            "log:component_interaction_persistent_handler_already_acknowledged custom_id=button-1 channel=123 user=42",
            events,
        )
        self.assertFalse(any(event.startswith("send:") for event in events))

    async def test_already_acknowledged_report_error_is_logged(self) -> None:
        deps, events = self.make_deps(report_error=AlreadyAckError("ack"))

        await unhandled_component.report_unhandled_component_interaction(
            FakeInteraction(),
            delay_sec=0,
            deps=deps,
        )

        self.assertIn(
            "log:component_interaction_unhandled_report_already_acknowledged custom_id=button-1 channel=123 user=42",
            events,
        )


if __name__ == "__main__":
    unittest.main()
