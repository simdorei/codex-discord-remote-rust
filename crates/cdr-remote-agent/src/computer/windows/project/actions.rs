use cdr_remote_protocol::output::ComputerWindowEntry;
use cdr_remote_protocol::request::{ComputerApp, ComputerMouseButton};
use cdr_windows_native::{
    NativeMouseButton, activate_window, capture_window_png, click_at, drag_at, post_close,
    press_device_keys, scroll_at, set_clipboard_text, type_device_text,
};

use super::WindowsProjectPlatform;
use crate::computer::windows::model::{change_summary, platform, same_window, screen_point};
use crate::computer::windows::policy::{require_notepad, require_project_keys};
use crate::computer::{ComputerCapture, ComputerError, ComputerIdentity, ComputerPlatform};

impl ComputerPlatform for WindowsProjectPlatform {
    fn list_windows(&self) -> Result<Vec<ComputerWindowEntry>, ComputerError> {
        self.prune()?;
        let window_ids = self.lock()?.keys().copied().collect::<Vec<_>>();
        window_ids
            .into_iter()
            .map(|id| self.resolve_owned(id).map(|window| window.entry))
            .collect()
    }

    fn screenshot(&self, window_id: u64) -> Result<ComputerCapture, ComputerError> {
        let before = self.resolve_owned(window_id)?;
        require_notepad(&before.identity)?;
        if !before.entry.active {
            return Err(platform(
                &"The active window changed. Take a fresh screenshot before continuing.",
            ));
        }
        let png = capture_window_png(
            window_id,
            u32::try_from(before.identity.width).map_err(|error| platform(&error))?,
            u32::try_from(before.identity.height).map_err(|error| platform(&error))?,
        )
        .map_err(|error| platform(&error))?;
        let after = self.resolve_owned(window_id)?;
        if !same_window(&before.identity, &after.identity) {
            return Err(platform(&format!(
                "window changed during capture: {}",
                change_summary(&before.identity, &after.identity)
            )));
        }
        if !after.entry.active {
            return Err(platform(&"The active window changed during capture."));
        }
        Ok(ComputerCapture {
            window: before.entry,
            identity: before.identity,
            png,
        })
    }

    fn activate(&self, window_id: u64) -> Result<ComputerWindowEntry, ComputerError> {
        let before = self.resolve_owned(window_id)?;
        activate_window(window_id, before.identity.process_id).map_err(|error| platform(&error))?;
        Ok(self.resolve_owned(window_id)?.entry)
    }

    fn launch(&self, app: ComputerApp) -> Result<(), ComputerError> {
        self.launch_owned(app)
    }

    fn click(
        &self,
        identity: &ComputerIdentity,
        x: u32,
        y: u32,
        button: ComputerMouseButton,
        count: u8,
    ) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        self.current(identity, true)?;
        let button = match button {
            ComputerMouseButton::Left => NativeMouseButton::Left,
            _ => return Err(platform(&"Only a window-bound left click is available.")),
        };
        let point = screen_point(identity, (x, y))?;
        click_at(
            identity.window_id,
            identity.process_id,
            point.0,
            point.1,
            button,
            count,
        )
        .map_err(|error| platform(&error))?;
        self.current(identity, false)
    }

    fn drag(
        &self,
        identity: &ComputerIdentity,
        start: (u32, u32),
        end: (u32, u32),
    ) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        self.current(identity, true)?;
        drag_at(
            identity.window_id,
            identity.process_id,
            screen_point(identity, start)?,
            screen_point(identity, end)?,
        )
        .map_err(|error| platform(&error))?;
        self.current(identity, false)
    }

    fn scroll(
        &self,
        identity: &ComputerIdentity,
        point: (u32, u32),
        delta: (i32, i32),
    ) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        self.current(identity, true)?;
        scroll_at(
            identity.window_id,
            identity.process_id,
            screen_point(identity, point)?,
            delta,
        )
        .map_err(|error| platform(&error))?;
        self.current(identity, false)
    }

    fn type_text(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        self.current(identity, true)?;
        type_device_text(identity.window_id, identity.process_id, text)
            .map_err(|error| platform(&error))?;
        self.current(identity, false)
    }

    fn press_keys(
        &self,
        identity: &ComputerIdentity,
        keys: &[String],
    ) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        require_project_keys(keys)?;
        self.current(identity, true)?;
        press_device_keys(identity.window_id, identity.process_id, keys)
            .map_err(|error| platform(&error))?;
        self.current(identity, false)
    }

    fn close(&self, identity: &ComputerIdentity) -> Result<(), ComputerError> {
        self.current(identity, true)?;
        post_close(identity.window_id, identity.process_id).map_err(|error| platform(&error))
    }

    fn set_clipboard(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError> {
        require_notepad(identity)?;
        self.current(identity, true)?;
        set_clipboard_text(text).map_err(|error| platform(&error))?;
        self.current(identity, true)
    }

    fn stop(&self) -> Result<(), ComputerError> {
        self.stop_owned()
    }
}
