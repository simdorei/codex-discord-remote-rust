from __future__ import annotations

import argparse


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Discord adapter for codex_desktop_bridge.py")
    _ = parser.add_argument(
        "--no-message-content",
        action="store_true",
        help="Disable prefix/plain-message handling and use slash commands only.",
    )
    return parser
