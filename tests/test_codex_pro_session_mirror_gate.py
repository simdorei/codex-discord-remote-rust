from __future__ import annotations

import unittest

import codex_pro_session_mirror_gate as mirror_gate


class ProSessionMirrorGateTests(unittest.TestCase):
    def tearDown(self) -> None:
        mirror_gate.reset_for_tests()

    def test_rejected_turn_requires_stable_rollout_before_discard(self) -> None:
        mirror_gate.hold("thread-1")
        mirror_gate.reject("thread-1")

        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.DISCARD)
        self.assertFalse(mirror_gate.discard_size_is_stable("thread-1", 100))
        self.assertFalse(mirror_gate.discard_size_is_stable("thread-1", 120))
        self.assertTrue(mirror_gate.discard_size_is_stable("thread-1", 120))

        mirror_gate.finish_discard("thread-1")
        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.OPEN)

    def test_successful_turn_opens_gate_immediately(self) -> None:
        mirror_gate.hold("thread-1")
        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.HOLD)

        mirror_gate.approve("thread-1")
        self.assertEqual(mirror_gate.mode("thread-1"), mirror_gate.GateMode.OPEN)


if __name__ == "__main__":
    _ = unittest.main()
