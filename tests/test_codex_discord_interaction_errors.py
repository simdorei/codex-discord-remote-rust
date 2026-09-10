from __future__ import annotations

import unittest

import codex_discord_interaction_errors as interaction_errors


class CodedInteractionError(Exception):
    def __init__(self, code: int) -> None:
        super().__init__()
        self.code: int = code


class FakeInteractionResponded(Exception):
    pass


class FakeErrors:
    InteractionResponded: type[BaseException] | None = FakeInteractionResponded


class MissingInteractionRespondedErrors:
    InteractionResponded: type[BaseException] | None = None


class InteractionErrorsTests(unittest.TestCase):
    def test_already_acknowledged_error_detects_discord_code(self) -> None:
        self.assertTrue(
            interaction_errors.is_interaction_already_acknowledged_error(
                CodedInteractionError(40060),
                interaction_responded_type=None,
            )
        )

    def test_already_acknowledged_error_detects_interaction_responded_type(self) -> None:
        self.assertTrue(
            interaction_errors.is_interaction_already_acknowledged_error(
                FakeInteractionResponded(),
                interaction_responded_type=FakeInteractionResponded,
            )
        )

    def test_already_acknowledged_error_detects_message_text(self) -> None:
        self.assertTrue(
            interaction_errors.is_interaction_already_acknowledged_error(
                RuntimeError("Interaction has already been acknowledged."),
                interaction_responded_type=None,
            )
        )

    def test_already_acknowledged_error_rejects_other_errors(self) -> None:
        self.assertFalse(
            interaction_errors.is_interaction_already_acknowledged_error(
                RuntimeError("rate limited"),
                interaction_responded_type=None,
            )
        )

    def test_get_interaction_responded_type_reads_discord_error(self) -> None:
        self.assertIs(
            interaction_errors.get_interaction_responded_type(FakeErrors()),
            FakeInteractionResponded,
        )

    def test_get_interaction_responded_type_allows_missing_type(self) -> None:
        self.assertIsNone(
            interaction_errors.get_interaction_responded_type(
                MissingInteractionRespondedErrors()
            )
        )

    def test_factory_detects_current_interaction_responded_type(self) -> None:
        checker = interaction_errors.make_interaction_already_acknowledged_error_checker(
            FakeErrors()
        )

        self.assertTrue(checker(FakeInteractionResponded()))


if __name__ == "__main__":
    _ = unittest.main()
