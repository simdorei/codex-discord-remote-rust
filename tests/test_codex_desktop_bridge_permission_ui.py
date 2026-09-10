# pyright: reportAny=false, reportAttributeAccessIssue=false, reportPrivateUsage=false, reportUnknownArgumentType=false, reportUnknownLambdaType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import subprocess
import unittest
from dataclasses import dataclass
from pathlib import Path
from typing import TypeAlias

import codex_desktop_bridge as bridge
import codex_desktop_bridge_permission_ui as permission_ui
from codex_thread_models import WindowInfo

PermissionCallValue: TypeAlias = str | int | float | bool | tuple[int, ...] | None
PermissionCall: TypeAlias = tuple[PermissionCallValue, ...]
RunProcessKwarg: TypeAlias = str | int | bool | dict[str, str]


class PermissionUiHappyTests(unittest.TestCase):
    def test_classifies_aliases_and_restores_clipboard_for_decline_message(self) -> None:
        self.assertEqual(permission_ui.classify_permission_approval_ui_reply("1"), ("accept", ""))
        self.assertEqual(permission_ui.classify_permission_approval_ui_reply("승인"), ("accept", ""))
        self.assertEqual(permission_ui.classify_permission_approval_ui_reply("2"), ("accept-remember", ""))
        self.assertEqual(permission_ui.classify_permission_approval_ui_reply("cancel"), ("cancel", ""))
        self.assertEqual(
            permission_ui.classify_permission_approval_ui_reply("too broad"),
            ("decline-message", "too broad"),
        )

        harness = _harness(clipboard="original")
        self.assertEqual(
            permission_ui.submit_permission_approval_via_ui("too broad", harness.deps),
            {
                "decision_action": "decline-message",
                "request_kind": "permission",
                "ui_result": "ACTION=decline-message",
            },
        )
        self.assertEqual(
            harness.calls,
            [
                ("clipboard-get",),
                ("focus", 7),
                ("clipboard-set", "too broad"),
                ("sleep", 0.1),
                ("hotkey", (17, 86)),
                ("sleep", 0.2),
                ("key", 13, False),
                ("key", 13, True),
                ("clipboard-set", "original"),
            ],
        )

    def test_script_paths_forward_env_and_preserve_bridge_wrappers(self) -> None:
        harness = _harness(stdout="ACTION=accept")
        self.assertEqual(
            permission_ui.submit_permission_approval_via_ui("1", harness.deps),
            {"decision_action": "accept", "request_kind": "permission", "ui_result": "ACTION=accept"},
        )
        self.assertEqual(
            harness.calls,
            [("focus", 7), ("read", "approval.ps1"), ("run", "approval.ps1", "scripts", 99)],
        )
        self.assertEqual(harness.environ["CODEX_APPROVAL_DECISION"], "accept")
        self.assertEqual(harness.environ["CODEX_APPROVAL_DECLINE_MESSAGE"], "")

        row = _harness(stdout="ACTION=accept-remember")
        self.assertEqual(
            permission_ui.submit_permission_approval_via_ui_row_select("2", row.deps),
            {
                "decision_action": "accept-remember",
                "request_kind": "permission",
                "ui_result": "ACTION=accept-remember",
            },
        )
        self.assertEqual(
            row.calls,
            [("focus", 7), ("read", "row_select.ps1"), ("run", "row_select.ps1", "scripts", 99)],
        )
        self.assertEqual(row.environ["CODEX_APPROVAL_DECISION"], "accept-remember")

        declined = _harness(stdout="ACTION=decline-message", clipboard="before")
        self.assertEqual(
            permission_ui.submit_permission_approval_via_ui_row_select("reason text", declined.deps),
            {
                "decision_action": "decline-message",
                "request_kind": "permission",
                "ui_result": "ACTION=decline-message",
            },
        )
        self.assertEqual(declined.environ["CODEX_APPROVAL_DECISION"], "decline-message")
        self.assertEqual(declined.environ["CODEX_APPROVAL_DECLINE_MESSAGE"], "reason text")
        self.assertEqual(declined.calls[-5:], [("hotkey", (17, 86)), ("sleep", 0.2), ("key", 13, False), ("key", 13, True), ("clipboard-set", "before")])

        original_deps = bridge._make_permission_ui_deps
        try:
            bridge._make_permission_ui_deps = lambda: _harness(stdout="ACTION=cancel").deps
            self.assertEqual(bridge.classify_permission_approval_ui_reply("2"), ("accept-remember", ""))
            self.assertEqual(
                bridge.submit_permission_approval_via_ui("cancel"),
                {"decision_action": "cancel", "request_kind": "permission", "ui_result": "ACTION=cancel"},
            )
        finally:
            bridge._make_permission_ui_deps = original_deps


class PermissionUiEdgeTests(unittest.TestCase):
    def test_rejects_malformed_and_unsupported_decisions(self) -> None:
        with self.assertRaisesRegex(permission_ui.PermissionApprovalReplyEmptyError, "Approval reply is empty"):
            _ = permission_ui.classify_permission_approval_ui_reply(" ")

        with self.assertRaisesRegex(permission_ui.PermissionApprovalDeclineMessageRequiredError, "Option 3 needs a decline message"):
            _ = permission_ui.classify_permission_approval_ui_reply("3")

        with self.assertRaisesRegex(permission_ui.UnsupportedPermissionApprovalDecisionError, "Unsupported permission approval decision: strange"):
            _ = permission_ui._script_action_for_decision("strange", allow_decline_message=True)

        with self.assertRaisesRegex(
            permission_ui.UnsupportedPermissionApprovalDecisionError,
            "Unsupported permission approval decision: decline-message",
        ):
            _ = permission_ui._script_action_for_decision("decline-message", allow_decline_message=False)

    def test_surfaces_subprocess_and_resource_failures(self) -> None:
        with self.assertRaisesRegex(permission_ui.PermissionApprovalUiSubmitError, "Permission approval UI submit failed: APPROVAL_CONTROL_NOT_FOUND"):
            _ = permission_ui.submit_permission_approval_via_ui(
                "1",
                _harness(returncode=6, stdout="APPROVAL_CONTROL_NOT_FOUND").deps,
            )

        with self.assertRaisesRegex(permission_ui.PermissionApprovalUiSubmitError, "Permission approval UI submit failed: denied"):
            _ = permission_ui.submit_permission_approval_via_ui(
                "1",
                _harness(returncode=4, stdout="", stderr="denied").deps,
            )

        with self.assertRaisesRegex(permission_ui.PermissionApprovalUiSubmitError, "Permission approval UI submit failed: exit=9"):
            _ = permission_ui.submit_permission_approval_via_ui(
                "1",
                _harness(returncode=9, stdout="", stderr="").deps,
            )

        def failing_run(args: list[str], **kwargs: RunProcessKwarg) -> subprocess.CompletedProcess[str]:
            _ = (args, kwargs)
            raise subprocess.SubprocessError("boom")

        with self.assertRaisesRegex(permission_ui.PermissionApprovalUiSubmitError, "Permission approval UI submit failed: boom"):
            _ = permission_ui.submit_permission_approval_via_ui("1", _harness(run=failing_run).deps)

        def missing_resource(_path: Path) -> str:
            raise OSError("missing resource")

        with self.assertRaisesRegex(OSError, "missing resource"):
            _ = permission_ui.submit_permission_approval_via_ui("1", _harness(read_text=missing_resource).deps)


@dataclass(frozen=True, slots=True)
class PermissionHarness:
    deps: permission_ui.PermissionUiDeps
    calls: list[PermissionCall]
    environ: dict[str, str]


def _harness(
    *,
    returncode: int = 0,
    stdout: str = "ACTION=accept",
    stderr: str = "",
    clipboard: str | None = None,
    run: permission_ui.RunProcess | None = None,
    read_text: permission_ui.ReadText | None = None,
) -> PermissionHarness:
    calls: list[PermissionCall] = []
    environ: dict[str, str] = {}
    approval_path = Path("approval.ps1")
    row_select_path = Path("row_select.ps1")

    def default_read_text(path: Path) -> str:
        calls.append(("read", path.name))
        return f"script::{path.name}"

    def run_process(args: list[str], **kwargs: RunProcessKwarg) -> subprocess.CompletedProcess[str]:
        script = str(args[-1])
        script_name = script.removeprefix("script::")
        cwd = kwargs.get("cwd")
        creationflags = kwargs.get("creationflags")
        calls.append(("run", script_name, cwd if isinstance(cwd, str) else "", creationflags if isinstance(creationflags, int) else 0))
        env = kwargs.get("env")
        if isinstance(env, dict):
            environ.update({key: str(value) for key, value in env.items()})
        return subprocess.CompletedProcess(args=args, returncode=returncode, stdout=stdout, stderr=stderr)

    def get_clipboard_text() -> str | None:
        calls.append(("clipboard-get",))
        return clipboard

    def set_clipboard_text(text: str) -> None:
        calls.append(("clipboard-set", text))

    deps = permission_ui.PermissionUiDeps(
        approval_script_path=approval_path,
        row_select_script_path=row_select_path,
        script_dir=Path("scripts"),
        read_text=read_text or default_read_text,
        get_clipboard_text=get_clipboard_text,
        set_clipboard_text=set_clipboard_text,
        find_codex_window=lambda: WindowInfo(hwnd=7, title="Codex", left=0, top=0, right=100, bottom=100),
        focus_window=lambda window: calls.append(("focus", window.hwnd)),
        send_hotkey=lambda *keys: calls.append(("hotkey", keys)),
        send_key_event=lambda vk, keyup=False: calls.append(("key", vk, keyup)),
        sleep=lambda seconds: calls.append(("sleep", seconds)),
        run_process=run or run_process,
        environ_copy=lambda: {},
        vk_control=17,
        vk_v=86,
        vk_return=13,
        create_no_window=99,
    )
    return PermissionHarness(deps=deps, calls=calls, environ=environ)


if __name__ == "__main__":
    unittest.main()
