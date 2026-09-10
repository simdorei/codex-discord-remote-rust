# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import unittest

import codex_desktop_bridge as bridge
import codex_desktop_bridge_protocol as bridge_protocol
import codex_desktop_bridge_thread_activation as thread_activation
from codex_thread_models import ThreadInfo


class UnexpectedSidebarError(Exception):
    pass


class ThreadActivationHappyTests(unittest.TestCase):
    def test_exact_thread_deep_link_is_primary_activation_path(self) -> None:
        opened: list[str] = []
        sidebar_calls: list[str] = []
        thread = _thread()

        bridge_protocol.open_codex_thread_deep_link(
            thread.id,
            start_file=opened.append,
        )
        result = thread_activation.activate_thread_in_ui(
            thread,
            _deps(
                activate_deep_link=lambda thread_id: opened.append(f"activated:{thread_id}"),
                activate_sidebar=lambda thread_name, project_name=None: sidebar_calls.append(thread_name) or "row",
                wait_activation=lambda thread, thread_name, timeout_sec=5.0: "copy-session-id",
            ),
        )

        self.assertEqual(opened, ["codex://threads/thread-1", "activated:thread-1"])
        self.assertEqual(result, "deeplink:codex [copy-session-id]")
        self.assertEqual(sidebar_calls, [])

    def test_wait_already_open_sidebar_and_bridge_wrappers(self) -> None:
        now = [0.0]
        header_calls: list[str] = []

        def verify_header(thread_name: str) -> str | None:
            header_calls.append(thread_name)
            if len(header_calls) >= 3:
                return "header"
            return None

        wait_deps = _deps(
            verify_header=verify_header,
            verify_thread=lambda _thread_id: None,
            now=lambda: now[0],
            sleep=lambda seconds: now.__setitem__(0, now[0] + seconds),
        )
        thread = _thread()

        self.assertEqual(
            thread_activation.wait_for_thread_activation(thread, "Thread", timeout_sec=2.0, deps=wait_deps),
            "header",
        )
        self.assertEqual(header_calls, ["Thread", "Thread", "Thread"])

        already_open = thread_activation.activate_thread_in_ui(
            thread,
            _deps(candidates=["Thread"], verify_header=lambda _name: "header"),
        )
        self.assertEqual(already_open, "already-open [header]")

        sidebar_calls: list[tuple[str, str | None]] = []
        sidebar_result = thread_activation.activate_thread_in_ui(
            thread,
            _deps(
                candidates=["Thread"],
                activate_sidebar=lambda thread_name, project_name=None: sidebar_calls.append((thread_name, project_name))
                or "Thread row",
                wait_activation=lambda thread, thread_name, timeout_sec=5.0: "copy-session-id",
            ),
        )
        self.assertEqual(sidebar_result, "sidebar:Thread row [copy-session-id]")
        self.assertEqual(sidebar_calls, [("Thread", "repo")])

        original_deps = bridge._make_thread_activation_deps
        try:
            bridge._make_thread_activation_deps = lambda: _deps(
                candidates=["Thread"],
                verify_header=lambda _name: None,
                verify_thread=lambda thread_id: "copy-session-id" if thread_id == "thread-1" else None,
            )
            self.assertEqual(bridge.verify_thread_in_ui(thread), "copy-session-id")
        finally:
            bridge._make_thread_activation_deps = original_deps


class ThreadActivationEdgeTests(unittest.TestCase):
    def test_timeout_candidate_fallback_no_label_and_unconfirmed_click(self) -> None:
        now = [0.0]
        self.assertIsNone(
            thread_activation.wait_for_thread_activation(
                _thread(),
                "Thread",
                timeout_sec=0.25,
                deps=_deps(now=lambda: now[0], sleep=lambda seconds: now.__setitem__(0, now[0] + seconds)),
            )
        )
        self.assertGreaterEqual(now[0], 0.25)

        attempts: list[str] = []

        def activate_sidebar(thread_name: str, project_name: str | None = None) -> str:
            _ = project_name
            attempts.append(thread_name)
            if thread_name == "First":
                raise thread_activation.ThreadActivationError("first failed")
            return "Second row"

        fallback_result = thread_activation.activate_thread_in_ui(
            _thread(),
            _deps(
                candidates=["First", "Second"],
                activate_sidebar=activate_sidebar,
                wait_activation=lambda thread, thread_name, timeout_sec=5.0: "header" if thread_name == "Second" else None,
            ),
        )
        self.assertEqual(fallback_result, "sidebar:Second row [header]")
        self.assertEqual(attempts, ["First", "Second"])

        with self.assertRaisesRegex(RuntimeError, "no usable UI label"):
            _ = thread_activation.activate_thread_in_ui(_thread(), _deps(candidates=[]))

        with self.assertRaisesRegex(RuntimeError, "did not confirm"):
            _ = thread_activation.activate_thread_in_ui(
                _thread(),
                _deps(
                    candidates=["Thread"],
                    activate_sidebar=lambda thread_name, project_name=None: "Thread row",
                    wait_activation=lambda thread, thread_name, timeout_sec=5.0: None,
                ),
            )

    def test_unexpected_sidebar_activation_failure_propagates(self) -> None:
        attempts: list[str] = []

        def activate_sidebar(thread_name: str, project_name: str | None = None) -> str:
            _ = project_name
            attempts.append(thread_name)
            raise UnexpectedSidebarError("sidebar dependency broke")

        with self.assertRaisesRegex(UnexpectedSidebarError, "sidebar dependency broke"):
            _ = thread_activation.activate_thread_in_ui(
                _thread(),
                _deps(candidates=["Thread"], activate_sidebar=activate_sidebar),
            )

        self.assertEqual(attempts, ["Thread"])


def _thread() -> ThreadInfo:
    return ThreadInfo(
        id="thread-1",
        title="Thread",
        cwd="C:/repo",
        updated_at=1,
        rollout_path="C:/repo/session.jsonl",
        model="gpt",
        reasoning_effort="high",
        tokens_used=1,
    )


def _deps(
    *,
    candidates: list[str] | None = None,
    verify_header: thread_activation.VerifyHeader | None = None,
    verify_thread: thread_activation.VerifyThread | None = None,
    activate_deep_link: thread_activation.ActivateThreadDeepLink | None = None,
    activate_sidebar: thread_activation.ActivateSidebar | None = None,
    wait_activation: thread_activation.WaitForThreadActivation | None = None,
    now: thread_activation.Clock | None = None,
    sleep: thread_activation.Sleep | None = None,
) -> thread_activation.ThreadActivationDeps:
    def default_activate_sidebar(thread_name: str, project_name: str | None = None) -> str:
        _ = (thread_name, project_name)
        return "Thread row"

    def default_activate_deep_link(_thread_id: str) -> None:
        raise OSError("deep link unavailable")

    def default_wait_activation(
        thread: ThreadInfo,
        thread_name: str,
        timeout_sec: float = 5.0,
    ) -> str | None:
        _ = (thread, thread_name, timeout_sec)
        return "header"

    return thread_activation.ThreadActivationDeps(
        get_thread_ui_name_candidates=lambda _thread: candidates if candidates is not None else ["Thread"],
        verify_active_thread_by_header=verify_header or (lambda _thread_name: None),
        verify_active_thread=verify_thread or (lambda _thread_id: None),
        activate_thread_deep_link=activate_deep_link or default_activate_deep_link,
        activate_thread_by_sidebar_v2=activate_sidebar or default_activate_sidebar,
        wait_for_thread_activation=wait_activation or default_wait_activation,
        now=now or (lambda: 0.0),
        sleep=sleep or (lambda _seconds: None),
    )


if __name__ == "__main__":
    unittest.main()
