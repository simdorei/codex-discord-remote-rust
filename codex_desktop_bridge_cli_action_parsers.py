# pyright: reportPrivateUsage=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse

from codex_desktop_bridge_cli_types import BridgeCommandHandlers


def add_optional_thread_ref(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "thread_ref",
        nargs="?",
        help="Optional workspace name, list index, `other`, or exact thread id.",
    )


def add_desktop_lifecycle_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    _add_codex_process_parsers(subparsers, handlers)
    _add_desktop_interaction_parsers(subparsers, common_parser, handlers)


def add_thread_action_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    _add_archive_action_parsers(subparsers, common_parser, handlers)
    _add_thread_navigation_parsers(subparsers, common_parser, handlers)
    _add_approval_reply_parser(subparsers, common_parser, handlers)


def _add_click_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--click-x-ratio", type=float, default=0.5)
    parser.add_argument("--click-y-offset", type=int, default=90)
    parser.add_argument("--click", action="store_true", help="Click inside the window before pasting.")


def _add_codex_process_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    handlers: BridgeCommandHandlers,
) -> None:
    discover_codex_parser = subparsers.add_parser(
        "discover_codex",
        help="Discover the Codex Desktop executable and store it in .env.",
    )
    discover_codex_parser.set_defaults(func=handlers.command_discover_codex)

    restart_codex_parser = subparsers.add_parser(
        "restart_codex",
        help="Restart the Codex Desktop app using the discovered executable path.",
    )
    restart_codex_parser.add_argument(
        "--stop-wait",
        type=float,
        default=1.0,
        help="Seconds to wait after terminating the old Codex Desktop process.",
    )
    restart_codex_parser.add_argument(
        "--start-wait",
        type=float,
        default=2.0,
        help="Seconds to wait after launching Codex Desktop before reporting status.",
    )
    restart_codex_parser.set_defaults(func=handlers.command_restart_codex)


def _add_desktop_interaction_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    focus_parser = subparsers.add_parser(
        "focus",
        help="Focus the Codex Desktop window.",
        parents=[common_parser],
    )
    focus_parser.add_argument("--click-x-ratio", type=float, default=0.5)
    focus_parser.add_argument("--click-y-offset", type=int, default=90)
    focus_parser.add_argument("--click", action="store_true", help="Also click inside the window after focusing.")
    focus_parser.set_defaults(func=handlers.command_focus)

    new_parser = subparsers.add_parser(
        "new",
        help="Create a new Codex thread through the local app-server sidecar.",
    )
    new_parser.add_argument("prompt", nargs="?", help="Optional first prompt for the new chat.")
    new_parser.add_argument("--abort", action="store_true", help="Abort the currently running Codex reply first.")
    new_parser.add_argument("--cwd", default=None, help="Working directory for the new thread. Defaults to the current shell cwd.")
    _add_click_options(new_parser)
    new_parser.add_argument(
        "--create-timeout",
        type=float,
        default=30.0,
        help="How long to wait for the newly created thread to appear in local state after sending a prompt.",
    )
    new_parser.set_defaults(func=handlers.command_new)


def _add_archive_action_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    archive_parser = subparsers.add_parser(
        "archive",
        help="Archive a thread through the local app-server sidecar.",
        parents=[common_parser],
    )
    add_optional_thread_ref(archive_parser)
    archive_parser.add_argument(
        "--timeout",
        type=float,
        default=8.0,
        help="How long to wait for the archived state to appear in local Codex state.",
    )
    archive_parser.add_argument(
        "--no-kill-codex-on-lock",
        action="store_true",
        help="Do not stop Codex processes and retry when thread/archive reports a Windows file lock.",
    )
    archive_parser.set_defaults(func=handlers.command_archive)

    delete_archive_parser = subparsers.add_parser(
        "delete_archive",
        help="Delete a locally archived thread and its local traces.",
    )
    delete_archive_parser.add_argument("thread_ref", help="Archived thread index, workspace ref, workspace name, or exact thread id.")
    delete_archive_parser.add_argument("--confirm", action="store_true", help="Actually delete the archived thread after previewing it.")
    delete_archive_parser.set_defaults(func=handlers.command_delete_archive)


def _add_thread_navigation_parsers(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    tail_parser = subparsers.add_parser(
        "tail",
        help="Tail session events for the selected thread.",
        parents=[common_parser],
    )
    tail_parser.add_argument("--timeout", type=float, default=0.0, help="0 means run forever.")
    tail_parser.add_argument("--only-new", action="store_true")
    tail_parser.set_defaults(func=handlers.command_tail)

    open_parser = subparsers.add_parser(
        "open",
        help="Select and open a thread in Codex Desktop without sending a prompt.",
        parents=[common_parser],
    )
    add_optional_thread_ref(open_parser)
    open_parser.add_argument("--abort", action="store_true", help="Abort the currently running Codex reply before switching threads.")
    open_parser.set_defaults(func=handlers.command_open)

    stop_parser = subparsers.add_parser(
        "stop",
        help="Stop the currently running Codex reply for a thread.",
        parents=[common_parser],
    )
    add_optional_thread_ref(stop_parser)
    stop_parser.set_defaults(func=handlers.command_stop)

    use_parser = subparsers.add_parser(
        "use",
        help="Select a default thread without opening it. Advanced.",
        parents=[common_parser],
    )
    use_parser.add_argument("thread_ref", nargs="?", help="Workspace name, list index, `other`, or exact thread id.")
    use_parser.add_argument("--clear", action="store_true", help="Clear the persisted selection.")
    use_parser.set_defaults(func=handlers.command_use)


def _add_approval_reply_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    handlers: BridgeCommandHandlers,
) -> None:
    approval_reply_parser = subparsers.add_parser(
        "approval_reply",
        help="Submit a pending approval reply for the selected thread.",
        parents=[common_parser],
    )
    approval_reply_parser.add_argument("answer", help="Approval reply text such as 1, 3, cancel, or a decline reason.")
    add_optional_thread_ref(approval_reply_parser)
    approval_reply_parser.add_argument("--timeout", type=float, default=8.0)
    approval_reply_parser.set_defaults(func=handlers.command_approval_reply)
