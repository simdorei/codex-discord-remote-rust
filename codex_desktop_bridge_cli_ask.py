# pyright: reportPrivateUsage=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
from collections.abc import Callable

CommandHandler = Callable[[argparse.Namespace], int]


def add_ask_parser(
    subparsers: argparse._SubParsersAction[argparse.ArgumentParser],
    common_parser: argparse.ArgumentParser,
    command_ask: CommandHandler,
) -> None:
    ask_parser = subparsers.add_parser(
        "ask",
        help="Send a prompt to the currently open Codex thread and stream the reply.",
        parents=[common_parser],
    )
    ask_parser.add_argument("prompt", help="Prompt text to send.")
    ask_parser.add_argument("--timeout", type=float, default=600.0)
    ask_parser.add_argument("--click-x-ratio", type=float, default=0.5)
    ask_parser.add_argument("--click-y-offset", type=int, default=90)
    ask_parser.add_argument("--click", action="store_true", help="Click inside the window before pasting.")
    ask_parser.add_argument("--dry-run", action="store_true")
    ask_parser.add_argument("--no-wait", dest="wait", action="store_false")
    ask_parser.add_argument("--background", dest="background", action="store_true", help="Return immediately and stream the reply in the background.")
    ask_parser.add_argument("--foreground", dest="background", action="store_false", help="Keep the current terminal occupied until the reply finishes.")
    ask_parser.add_argument("--include-commentary", dest="include_commentary", action="store_true")
    ask_parser.add_argument("--no-commentary", dest="include_commentary", action="store_false")
    ask_parser.add_argument("--stream", dest="stream", action="store_true", help="Stream commentary while a reply is in progress.")
    ask_parser.add_argument("--no-stream", dest="stream", action="store_false", help="Do not stream reply text; only print ready.")
    ask_parser.add_argument("--force-while-busy", action="store_true")
    ask_parser.add_argument("--ipc", dest="ipc", action="store_true", help="Send the prompt through Codex IPC without UI paste. Default behavior.")
    ask_parser.add_argument("--ui", dest="ipc", action="store_false", help="Use the legacy UI paste path. This can move the Codex window to the foreground.")
    ask_parser.add_argument("--sidecar", dest="sidecar", action="store_true", help="Send the prompt through Codex app-server turn/start for an explicit thread.")
    ask_parser.add_argument("--ipc-recover-ui", action="store_true", help="If background IPC cannot find the target thread owner, reactivate the thread in the Codex UI and retry.")
    ask_parser.add_argument("--no-fallback", action="store_true", help="Compatibility flag; fallback delivery is disabled.")
    ask_parser.add_argument("--switch-thread", dest="switch_thread", action="store_true", help="Switch the Codex UI to the target thread before sending.")
    ask_parser.add_argument("--no-switch-thread", dest="switch_thread", action="store_false", help="Do not switch threads. Send to the currently open Codex thread only. Default behavior.")
    ask_parser.set_defaults(
        func=command_ask,
        wait=True,
        background=False,
        ipc=True,
        sidecar=False,
        ipc_recover_ui=False,
        no_fallback=True,
        switch_thread=False,
        stream=False,
        include_commentary=False,
    )
