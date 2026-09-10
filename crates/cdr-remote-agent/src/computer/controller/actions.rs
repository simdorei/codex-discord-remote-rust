use cdr_remote_protocol::output::{ComputerActionName, ComputerOutput};
use cdr_remote_protocol::request::{ComputerMouseButton, ComputerRequest};

use super::ComputerController;
use super::operations::action;
use crate::computer::ComputerError;

impl ComputerController {
    pub(super) fn execute_observed(
        &self,
        request: &ComputerRequest,
    ) -> Result<ComputerOutput, ComputerError> {
        match request {
            ComputerRequest::ComputerClick {
                window_id,
                observation_id,
                x,
                y,
                button,
                click_count,
            } => self.click(*window_id, observation_id, (*x, *y), *button, *click_count),
            ComputerRequest::ComputerDrag {
                window_id,
                observation_id,
                start_x,
                start_y,
                end_x,
                end_y,
            } => self.drag(
                *window_id,
                observation_id,
                [(*start_x, *start_y), (*end_x, *end_y)],
            ),
            ComputerRequest::ComputerScroll {
                window_id,
                observation_id,
                x,
                y,
                delta_x,
                delta_y,
            } => self.scroll(*window_id, observation_id, (*x, *y), (*delta_x, *delta_y)),
            ComputerRequest::ComputerTypeText {
                window_id,
                observation_id,
                text,
            } => self.type_text(*window_id, observation_id, text),
            ComputerRequest::ComputerPressKeys {
                window_id,
                observation_id,
                keys,
            } => self.press_keys(*window_id, observation_id, keys),
            ComputerRequest::ComputerClose {
                window_id,
                observation_id,
            } => self.close(*window_id, observation_id),
            ComputerRequest::ComputerSetClipboard {
                window_id,
                observation_id,
                text,
            } => self.set_clipboard(*window_id, observation_id, text),
            _ => unreachable!("unobserved request was handled by the caller"),
        }
    }

    fn click(
        &self,
        window_id: u64,
        observation_id: &str,
        point: (u32, u32),
        button: ComputerMouseButton,
        count: u8,
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[point])?;
        self.platform
            .click(&identity, point.0, point.1, button, count)?;
        Ok(action(
            ComputerActionName::Click,
            Some(window_id),
            "Click sent.",
        ))
    }

    fn drag(
        &self,
        window_id: u64,
        observation_id: &str,
        points: [(u32, u32); 2],
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &points)?;
        self.platform.drag(&identity, points[0], points[1])?;
        Ok(action(
            ComputerActionName::Drag,
            Some(window_id),
            "Drag sent.",
        ))
    }

    fn scroll(
        &self,
        window_id: u64,
        observation_id: &str,
        point: (u32, u32),
        delta: (i32, i32),
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[point])?;
        self.platform.scroll(&identity, point, delta)?;
        Ok(action(
            ComputerActionName::Scroll,
            Some(window_id),
            "Scroll sent.",
        ))
    }

    fn type_text(
        &self,
        window_id: u64,
        observation_id: &str,
        text: &str,
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[])?;
        self.platform.type_text(&identity, text)?;
        Ok(action(
            ComputerActionName::TypeText,
            Some(window_id),
            "Text typed.",
        ))
    }

    fn press_keys(
        &self,
        window_id: u64,
        observation_id: &str,
        keys: &[String],
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[])?;
        self.platform.press_keys(&identity, keys)?;
        Ok(action(
            ComputerActionName::PressKeys,
            Some(window_id),
            "Keys pressed.",
        ))
    }

    fn close(&self, window_id: u64, observation_id: &str) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[])?;
        self.platform.close(&identity)?;
        Ok(action(
            ComputerActionName::Close,
            Some(window_id),
            "Close requested.",
        ))
    }

    fn set_clipboard(
        &self,
        window_id: u64,
        observation_id: &str,
        text: &str,
    ) -> Result<ComputerOutput, ComputerError> {
        let identity = self.consume(observation_id, window_id, &[])?;
        self.platform.set_clipboard(&identity, text)?;
        Ok(action(
            ComputerActionName::SetClipboard,
            Some(window_id),
            "Clipboard text set.",
        ))
    }
}
