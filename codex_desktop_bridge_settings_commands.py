from __future__ import annotations

from collections.abc import Callable, Mapping
from dataclasses import dataclass
from typing import Protocol

from codex_thread_models import ThreadInfo
from codex_thread_settings import JsonObject, JsonValue, ThreadSettingsUpdate


class SettingsSidecar(Protocol):
    def list_models(self) -> JsonObject: ...

    def resume_thread(self, thread_id: str) -> JsonObject: ...

    def update_thread_settings(self, thread_id: str, settings: ThreadSettingsUpdate) -> JsonObject: ...

    def close(self) -> None: ...


class BuildThreadSettingsUpdate(Protocol):
    def __call__(
        self,
        model: str | None,
        reasoning: str | None,
        speed: str | None,
        *,
        model_catalog: Mapping[str, JsonValue] | None = None,
        current_model: str | None = None,
    ) -> ThreadSettingsUpdate: ...


class RememberThreadSettings(Protocol):
    def __call__(
        self,
        thread_id: str,
        *,
        model: str | None,
        reasoning: str | None,
        speed: str | None,
    ) -> None: ...


ChooseThread = Callable[[str | None, str | None], ThreadInfo]
FormatSettingsOptions = Callable[[Mapping[str, JsonValue], str], str]
FormatTitlePreview = Callable[[str], str]
GetSavedThreadSettings = Callable[[str], Mapping[str, str]]
NewSidecar = Callable[[], SettingsSidecar]
PrintLine = Callable[[str], None]
ResolveThreadRef = Callable[[str], ThreadInfo]


@dataclass(frozen=True, slots=True)
class SettingsCommandDeps:
    choose_thread: ChooseThread
    resolve_thread_ref: ResolveThreadRef
    new_sidecar: NewSidecar
    build_thread_settings_update: BuildThreadSettingsUpdate
    remember_thread_settings: RememberThreadSettings
    get_saved_thread_settings: GetSavedThreadSettings
    format_title_preview: FormatTitlePreview
    format_settings_options: FormatSettingsOptions
    print_line: PrintLine


def run_settings_command(
    *,
    thread_ref: str,
    thread_id: str | None,
    cwd: str | None,
    model: str | None,
    reasoning: str | None,
    speed: str | None,
    deps: SettingsCommandDeps,
) -> None:
    thread = deps.resolve_thread_ref(thread_ref) if thread_ref else deps.choose_thread(thread_id, cwd)

    sidecar = deps.new_sidecar()
    try:
        model_catalog = sidecar.list_models()
        settings = deps.build_thread_settings_update(
            model,
            reasoning,
            speed,
            model_catalog=model_catalog,
            current_model=thread.model,
        )
        _ = sidecar.resume_thread(thread.id)
        _ = sidecar.update_thread_settings(thread.id, settings)
    finally:
        sidecar.close()

    deps.remember_thread_settings(thread.id, model=model, reasoning=reasoning, speed=speed)
    saved_settings = deps.get_saved_thread_settings(thread.id)
    deps.print_line(f"target_thread: {thread.id}")
    deps.print_line(f"title: {deps.format_title_preview(thread.title)}")
    deps.print_line(f"cwd: {thread.cwd}")
    deps.print_line(f"model: {saved_settings.get('model', thread.model)}")
    deps.print_line(f"reasoning: {saved_settings.get('reasoning', thread.reasoning_effort)}")
    deps.print_line(f"speed: {saved_settings.get('speed', '-')}")
    deps.print_line("transport: local-sidecar thread/settings/update")


def run_settings_options_command(*, field: str, deps: SettingsCommandDeps) -> None:
    sidecar = deps.new_sidecar()
    try:
        model_catalog = sidecar.list_models()
    finally:
        sidecar.close()
    deps.print_line(deps.format_settings_options(model_catalog, field))
