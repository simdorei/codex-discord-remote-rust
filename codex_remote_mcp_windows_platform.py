from __future__ import annotations

import logging
import threading
from collections.abc import Callable
from typing import final

from codex_remote_mcp_computer_contracts import ComputerActionPermit, ComputerCapture
from codex_remote_mcp_computer_errors import ComputerControlError
from codex_remote_mcp_windows_close import close_owned_window
from codex_remote_mcp_windows_input import (
    click_window,
    drag_window,
    press_window_keys,
    scroll_window,
    set_clipboard_text,
    type_window_text,
)
from codex_remote_mcp_windows_launch import (
    launch_allowed_app,
    retry_failed_launch_cleanup,
    stop_owned_process,
)
from codex_remote_mcp_windows_launch_cleanup import remove_temporary_profile
from codex_remote_mcp_windows_launch_types import (
    ApplicationLaunchCleanupError,
    FailedLaunchCleanup,
    OwnedProcess,
)
from codex_remote_mcp_windows_platform_lifecycle import (
    OwnedActionPermit,
    OwnedLaunch,
    OwnedWindow,
    cleanup_owned_applications,
    prune_exited_launches,
    require_live_owned_launch,
)
from codex_remote_mcp_windows_screenshot import capture_resolved_window
from codex_remote_mcp_windows_windows import (
    ResolvedWindow,
    activate_window,
    require_matching_active_window,
    resolve_allowed_window,
)
from simdorei_mcp_common.operation_outputs import ComputerWindowEntry

LOGGER = logging.getLogger(__name__)


@final
class WindowsComputerPlatform:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._owned: dict[int, OwnedWindow] = {}
        self._launched: dict[int, OwnedLaunch] = {}
        self._failed_launches: list[FailedLaunchCleanup] = []

    def list_windows(self) -> tuple[ComputerWindowEntry, ...]:
        self._prune_exited_processes()
        with self._lock:
            window_ids = tuple(self._owned)
        windows: list[ComputerWindowEntry] = []
        for window_id in window_ids:
            try:
                windows.append(self._resolve_owned(window_id).entry)
            except ComputerControlError:
                with self._lock:
                    _ = self._owned.pop(window_id, None)
        return tuple(windows)

    def stop(self, *, deadline_monotonic: float | None = None) -> None:
        with self._lock:
            launched = tuple(self._launched.values())
            failed_launches = tuple(self._failed_launches)
        if deadline_monotonic is None:
            stop_process = stop_owned_process
            remove_profile = remove_temporary_profile
            retry_cleanup = retry_failed_launch_cleanup
        else:
            def stop_process(
                window_id: int,
                process: OwnedProcess,
                process_path: str,
            ) -> None:
                stop_owned_process(
                    window_id,
                    process,
                    process_path,
                    deadline_monotonic=deadline_monotonic,
                )

            def remove_profile(directory: str | None) -> None:
                remove_temporary_profile(
                    directory,
                    deadline_monotonic=deadline_monotonic,
                )

            def retry_cleanup(cleanup: FailedLaunchCleanup) -> None:
                retry_failed_launch_cleanup(
                    cleanup,
                    deadline_monotonic=deadline_monotonic,
                )
        outcome = cleanup_owned_applications(
            launched,
            failed_launches,
            stop_process,
            remove_profile,
            retry_cleanup,
            deadline_monotonic=deadline_monotonic,
        )
        with self._lock:
            for cleanup in outcome.completed_failed_launches:
                self._failed_launches = [
                    current
                    for current in self._failed_launches
                    if current is not cleanup
                ]
            for launch in outcome.completed_launches:
                if self._launched.get(launch.owner.process_id) == launch:
                    del self._launched[launch.owner.process_id]
                if self._owned.get(launch.window_id) == launch.owner:
                    del self._owned[launch.window_id]
        if outcome.failures:
            failure = outcome.failures[0]
            LOGGER.warning(
                "remote_owned_process_stop_failed failures=%s error=%s",
                len(outcome.failures),
                failure.reason,
            )
            raise failure

    def screenshot(self, window_id: int) -> ComputerCapture:
        capture = capture_resolved_window(self._resolve_owned(window_id))
        _ = self._resolve_owned(window_id)
        return capture

    def activate(self, window_id: int) -> ComputerWindowEntry:
        _ = self._resolve_owned(window_id)
        return activate_window(window_id)

    def launch(
        self,
        app: str,
        *,
        ensure_active: Callable[[], None] | None = None,
    ) -> None:
        self._prune_exited_processes()
        process_name = f"{app}.exe" if app == "chrome" else "notepad.exe"
        with self._lock:
            already_launched = any(
                launch.owner.process_name == process_name
                for launch in self._launched.values()
            )
            cleanup_pending = any(
                cleanup.app == app for cleanup in self._failed_launches
            )
        if already_launched or cleanup_pending:
            raise ComputerControlError(
                f"This session already has a launched {app} window."
            )
        try:
            if ensure_active is None:
                launched = launch_allowed_app(app)
            else:
                launched = launch_allowed_app(app, ensure_active=ensure_active)
        except ApplicationLaunchCleanupError as exc:
            with self._lock:
                self._failed_launches.append(exc.cleanup)
            raise
        resolved = launched.window
        owner = OwnedWindow(
            process_id=resolved.identity.process_id,
            process_path=resolved.identity.process_path,
            process_name=resolved.entry.process_name,
            process=launched.process,
        )
        with self._lock:
            self._owned[resolved.entry.window_id] = owner
            self._launched[owner.process_id] = OwnedLaunch(
                window_id=resolved.entry.window_id,
                owner=owner,
                process=launched.process,
                temporary_profile=launched.temporary_profile,
            )

    def close(self, permit: ComputerActionPermit) -> None:
        owned = self._owned_action_permit(permit)
        owned.require_active()
        _ = require_matching_active_window(permit.identity)
        launch = owned.require_owned_launch()
        _ = require_matching_active_window(permit.identity)
        close_owned_window(permit.identity, launch)
        with self._lock:
            current = self._owned.get(permit.identity.window_id)
            if current is not launch.owner:
                raise ComputerControlError("The launched window identity changed.")
            del self._owned[permit.identity.window_id]

    def _prune_exited_processes(self) -> None:
        with self._lock:
            launched = tuple(self._launched.values())
        exited = prune_exited_launches(launched, remove_temporary_profile)
        if not exited:
            return
        with self._lock:
            for launch in exited:
                process_id = launch.owner.process_id
                if self._launched.get(process_id) == launch:
                    del self._launched[process_id]
                if self._owned.get(launch.window_id) == launch.owner:
                    del self._owned[launch.window_id]

    def click(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        button: str,
        click_count: int,
    ) -> None:
        click_window(self._owned_action_permit(permit), x, y, button, click_count)

    def drag(
        self,
        permit: ComputerActionPermit,
        start_x: int,
        start_y: int,
        end_x: int,
        end_y: int,
    ) -> None:
        drag_window(self._owned_action_permit(permit), start_x, start_y, end_x, end_y)

    def scroll(
        self,
        permit: ComputerActionPermit,
        x: int,
        y: int,
        delta_x: int,
        delta_y: int,
    ) -> None:
        scroll_window(self._owned_action_permit(permit), x, y, delta_x, delta_y)

    def type_text(self, permit: ComputerActionPermit, text: str) -> None:
        type_window_text(self._owned_action_permit(permit), text)

    def press_keys(self, permit: ComputerActionPermit, keys: tuple[str, ...]) -> None:
        press_window_keys(self._owned_action_permit(permit), keys)

    def set_clipboard(self, permit: ComputerActionPermit, text: str) -> None:
        owned = self._owned_action_permit(permit)
        owned.require_active()
        current = require_matching_active_window(permit.identity)
        if current.entry.process_name.casefold() != "notepad.exe":
            raise ComputerControlError(
                "Clipboard writes are available only in Notepad."
            )
        set_clipboard_text(text)
        owned.require_active()
        _ = require_matching_active_window(permit.identity)

    def _resolve_owned(self, window_id: int) -> ResolvedWindow:
        with self._lock:
            owner = self._owned.get(window_id)
            launch = self._launched.get(owner.process_id) if owner is not None else None
        if owner is None:
            raise ComputerControlError(
                "Only a window launched by this ChatGPT session can be controlled."
            )
        _ = require_live_owned_launch(owner, launch)
        current = resolve_allowed_window(window_id)
        if (
            current.identity.process_id != owner.process_id
            or current.identity.process_path != owner.process_path
            or current.entry.process_name != owner.process_name
        ):
            raise ComputerControlError("The launched window identity changed.")
        return current

    def _require_owned_identity(self, permit: ComputerActionPermit) -> OwnedLaunch:
        with self._lock:
            owner = self._owned.get(permit.identity.window_id)
            launch = self._launched.get(owner.process_id) if owner is not None else None
        if (
            owner is None
            or owner.process_id != permit.identity.process_id
            or owner.process_path != permit.identity.process_path
        ):
            raise ComputerControlError(
                "Only a window launched by this ChatGPT session can be controlled."
            )
        return require_live_owned_launch(owner, launch)

    def _owned_action_permit(
        self,
        permit: ComputerActionPermit,
    ) -> OwnedActionPermit:
        return OwnedActionPermit(
            source=permit,
            verify_owned_process=lambda: self._require_owned_identity(permit),
        )
