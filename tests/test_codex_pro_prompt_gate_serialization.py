from __future__ import annotations

from threading import Event, Thread
import unittest

import codex_discord_prompt_transport as prompt_transport
import codex_pro_session_mirror_gate as mirror_gate
from tests.test_codex_discord_prompt_transport import build_deps


class ProPromptGateSerializationTests(unittest.TestCase):
    def tearDown(self) -> None:
        mirror_gate.reset_for_tests()

    def test_next_prompt_waits_until_rejected_pro_output_is_discarded(self) -> None:
        resident_started = Event()
        completed = Event()

        def resident(_prompt: str, _target_thread_id: str | None) -> tuple[int, str]:
            resident_started.set()
            return 0, "delivered"

        def run_prompt() -> None:
            _ = prompt_transport.run_transport_prompt_no_wait(
                "ordinary prompt",
                "thread-1",
                build_deps(run_resident_prompt_no_wait=resident),
            )
            completed.set()

        mirror_gate.reject("thread-1")
        worker = Thread(target=run_prompt)
        worker.start()

        self.assertFalse(resident_started.wait(timeout=0.1))
        mirror_gate.finish_discard("thread-1")
        self.assertTrue(completed.wait(timeout=1.0))
        worker.join(timeout=1.0)
        self.assertFalse(worker.is_alive())
        self.assertTrue(resident_started.is_set())


if __name__ == "__main__":
    _ = unittest.main()
