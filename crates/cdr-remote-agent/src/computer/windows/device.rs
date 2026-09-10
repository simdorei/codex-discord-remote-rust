use cdr_remote_protocol::output::ComputerWindowEntry;
use cdr_remote_protocol::request::{ComputerApp, ComputerMouseButton};
use cdr_windows_native::{
    NativeMouseButton, activate_window, capture_window_png, click_at, drag_at, post_close,
    press_device_keys, scroll_at, set_clipboard_text, type_device_text,
};

use super::model::{
    change_summary, list_device_windows, platform, require_matching, resolve_device, same_window,
    screen_point,
};
use super::project::WindowsProjectPlatform;
use crate::computer::{ComputerCapture, ComputerError, ComputerIdentity, ComputerPlatform};

pub(super) struct WindowsDevicePlatform {
    launched: WindowsProjectPlatform,
}

impl WindowsDevicePlatform {
    pub fn new() -> Self {
        Self {
            launched: WindowsProjectPlatform::new(),
        }
    }

    fn current(identity: &ComputerIdentity, include_title: bool) -> Result<(), ComputerError> {
        require_matching(
            identity,
            &resolve_device(identity.window_id)?,
            include_title,
        )
    }
}

impl ComputerPlatform for WindowsDevicePlatform {
    fn list_windows(&self) -> Result<Vec<ComputerWindowEntry>, ComputerError> {
        list_device_windows()
    }

    fn screenshot(&self, window_id: u64) -> Result<ComputerCapture, ComputerError> {
        let before = resolve_device(window_id)?;
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
        let after = resolve_device(window_id)?;
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
        let before = resolve_device(window_id)?;
        activate_window(window_id, before.identity.process_id).map_err(|error| platform(&error))?;
        let after = resolve_device(window_id)?;
        if !after.entry.active {
            return Err(platform(
                &"Windows did not allow the window to become active.",
            ));
        }
        Ok(after.entry)
    }

    fn launch(&self, app: ComputerApp) -> Result<(), ComputerError> {
        self.launched.launch(app)
    }

    fn click(
        &self,
        identity: &ComputerIdentity,
        x: u32,
        y: u32,
        button: ComputerMouseButton,
        count: u8,
    ) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        let point = screen_point(identity, (x, y))?;
        click_at(
            identity.window_id,
            identity.process_id,
            point.0,
            point.1,
            mouse_button(button),
            count,
        )
        .map_err(|error| platform(&error))?;
        Self::current(identity, false)
    }

    fn drag(
        &self,
        identity: &ComputerIdentity,
        start: (u32, u32),
        end: (u32, u32),
    ) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        drag_at(
            identity.window_id,
            identity.process_id,
            screen_point(identity, start)?,
            screen_point(identity, end)?,
        )
        .map_err(|error| platform(&error))?;
        Self::current(identity, false)
    }

    fn scroll(
        &self,
        identity: &ComputerIdentity,
        point: (u32, u32),
        delta: (i32, i32),
    ) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        scroll_at(
            identity.window_id,
            identity.process_id,
            screen_point(identity, point)?,
            delta,
        )
        .map_err(|error| platform(&error))?;
        Self::current(identity, false)
    }

    fn type_text(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        type_device_text(identity.window_id, identity.process_id, text)
            .map_err(|error| platform(&error))?;
        Self::current(identity, false)
    }

    fn press_keys(
        &self,
        identity: &ComputerIdentity,
        keys: &[String],
    ) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        press_device_keys(identity.window_id, identity.process_id, keys)
            .map_err(|error| platform(&error))?;
        Self::current(identity, false)
    }

    fn close(&self, identity: &ComputerIdentity) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        post_close(identity.window_id, identity.process_id).map_err(|error| platform(&error))
    }

    fn set_clipboard(&self, identity: &ComputerIdentity, text: &str) -> Result<(), ComputerError> {
        Self::current(identity, true)?;
        set_clipboard_text(text).map_err(|error| platform(&error))?;
        Self::current(identity, true)
    }

    fn stop(&self) -> Result<(), ComputerError> {
        self.launched.stop()
    }
}

fn mouse_button(value: ComputerMouseButton) -> NativeMouseButton {
    match value {
        ComputerMouseButton::Left => NativeMouseButton::Left,
        ComputerMouseButton::Right => NativeMouseButton::Right,
        ComputerMouseButton::Middle => NativeMouseButton::Middle,
    }
}
