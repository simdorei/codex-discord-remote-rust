from typing import Final


MCP_INSTRUCTIONS: Final = (
    "Use this connector directly in normal Chat with Pro reasoning; do not switch "
    "to Work. For Codex tickets, call `list_devices`, then call `select_device` once "
    "with the supplied device ID and absolute project folder. This PC mode is the "
    "default and needs no project scope. Confirm the active PC and folder with "
    "`device_info`. Read existing project files before changing them and pass their "
    "SHA-256 values to `file_apply_patch`. File tools remain bound to the selected "
    "working folder. "
    "Use `retrieve_image` when visual inspection is needed. `terminal_exec` may run "
    "unrestricted user-authorized PowerShell, cmd, sh, or bash text, launch child "
    "processes, and use an explicit absolute working directory outside the selected "
    "project. Use `terminal_window_open`, `terminal_window_list`, "
    "`terminal_window_capture`, `terminal_window_activate`, `terminal_window_type`, "
    "`terminal_window_keys`, `terminal_window_interrupt`, and "
    "`terminal_window_close` to control visible terminal windows created by this "
    "ChatGPT session. Terminal commands may perform verification and Git commit or "
    "push; review `repo_status` and `show_changes` first. PC mode can use the selected "
    "absolute working folder plus existing visible Windows app windows. Use "
    "`set_working_directory` only when the user explicitly changes folders. Every "
    "input action consumes one fresh "
    "screenshot observation. Windows secure-desktop and locked-session surfaces remain "
    "unavailable. Never "
    "use file, terminal, or computer tools to read, request, enter, extract, or "
    "transmit passwords, OTP codes, API keys, tokens, cookies, or other credentials. "
    "Never enter passwords or OTPs; leave login, CAPTCHA, and other direct identity "
    "checks to the user."
)


__all__ = ["MCP_INSTRUCTIONS"]
