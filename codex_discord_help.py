"""Help text for the Discord bridge commands."""

from __future__ import annotations


def build_help(*, qa_commands_enabled: bool, host_commands_enabled: bool = False) -> str:
    slash_commands = [
        "/help",
        "/list",
        "/archived_list",
        "/use",
        "/status",
        "/settings",
        "/doctor",
        "/where",
        "/context",
        "/usage",
        "/runners",
        "/retract",
        "/mirror_check",
        "/bridge_sync",
        "/new",
        "/ask",
        "/interview",
        "/ask_ipc",
    ]
    lines = [
        "Codex Discord commands",
        "!help",
        "!list [limit]",
        "!archived_list [limit]  (alias: !archive_list)",
        "!use <ref>",
        "!open <ref>",
        "!open_abort <ref>",
        "!stop [ref]  (stop the running Codex reply for the mapped or selected thread)",
        "!resume [ref]  (restore a mapped or selected Codex thread; does not resend the prompt)",
        "!status [ref]",
        "!settings [ref] --model <model> --effort <effort> --speed <speed>  (alias: !setting; omit a value to list options)",
        "!doctor",
        "!discover_codex",
        "!restart_codex",
        "!chatid",
        "!where",
        "!context [all|refresh [limit]]",
        "!usage [days]",
        "!runners",
        "!resources  (alias: !system)",
        "!retract [ref]  (remove your latest queued ask)",
        "!bridge sync [limit]",
        "!mirror sync",
        "!mirror list [limit]",
        "!mirror check [limit]",
        "!approval",
        "!archive [ref]",
        "!archive-used <threshold>",
        "!delete_archive <ref>",
        "!confirm_delete_archive <ref>",
        "!new <prompt>  (create a new Codex thread with the first prompt)",
        "!interview <request>  (Gajae-style clarify before implementation)",
        "",
        "Plain messages in mirrored Discord threads are sent to that Codex thread.",
    ]
    if host_commands_enabled:
        lines.insert(lines.index("!chatid"), "!reset_pc confirm")
    if qa_commands_enabled:
        lines.insert(lines.index("!approval"), "!qa buttons")
        lines.insert(lines.index("!qa buttons") + 1, "!steer <prompt>  (QA-only text path for Steer now)")
        slash_commands.insert(slash_commands.index("/new"), "/qa_buttons")
    lines.append(f"Slash commands: {', '.join(slash_commands)}.")
    return "\n".join(lines)
