from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

from codex_thread_models import WindowInfo


ClickWindow = Callable[[WindowInfo, float, int], tuple[int, int]]
EnsureComposerFocus = Callable[[], bool]
FindCodexWindow = Callable[[], WindowInfo]
FocusWindow = Callable[[WindowInfo], None]
GetClipboardText = Callable[[], str | None]
PrintLine = Callable[[str], None]
SendHotkey = Callable[..., None]
SendKeyEvent = Callable[[int, bool], None]
SetClipboardText = Callable[[str], None]
Sleep = Callable[[float], None]


class PromptClipboardMismatchError(RuntimeError):
    pass


@dataclass(frozen=True, slots=True)
class PromptSenderDeps:
    find_codex_window: FindCodexWindow
    focus_window: FocusWindow
    ensure_codex_composer_focus: EnsureComposerFocus
    click_window: ClickWindow
    send_hotkey: SendHotkey
    send_key_event: SendKeyEvent
    set_clipboard_text: SetClipboardText
    get_clipboard_text: GetClipboardText
    sleep: Sleep
    print_line: PrintLine
    vk_control: int
    vk_a: int
    vk_back: int
    vk_v: int
    vk_return: int


def send_prompt_to_codex(
    prompt: str,
    click_x_ratio: float,
    click_y_offset: int,
    skip_click: bool,
    deps: PromptSenderDeps,
) -> WindowInfo:
    window = deps.find_codex_window()
    deps.focus_window(window)
    composer_focused = deps.ensure_codex_composer_focus()
    if not skip_click:
        _ = deps.click_window(window, click_x_ratio, click_y_offset)
        composer_focused = deps.ensure_codex_composer_focus() or composer_focused
    if composer_focused:
        deps.send_hotkey(deps.vk_control, deps.vk_a)
        deps.send_key_event(deps.vk_back, False)
        deps.send_key_event(deps.vk_back, True)
        deps.sleep(0.05)
    deps.set_clipboard_text(prompt)
    if deps.get_clipboard_text() != prompt:
        deps.sleep(0.05)
        if deps.get_clipboard_text() != prompt:
            raise PromptClipboardMismatchError("Clipboard did not contain the prompt after setting it.")
    deps.send_hotkey(deps.vk_control, deps.vk_v)
    deps.send_key_event(deps.vk_return, False)
    deps.send_key_event(deps.vk_return, True)
    if not composer_focused:
        deps.print_line("[warning] Composer focus was not confirmed before paste.")
    return window
