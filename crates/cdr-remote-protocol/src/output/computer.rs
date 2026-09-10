use cdr_core::{Validate, ValidationError, ValidationResult, length};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerWindowEntry {
    pub window_id: u64,
    pub title: String,
    pub process_name: String,
    pub left: i64,
    pub top: i64,
    pub width: u64,
    pub height: u64,
    pub active: bool,
}

impl Validate for ComputerWindowEntry {
    fn validate(&self) -> ValidationResult {
        if self.window_id == 0 || self.width == 0 || self.height == 0 {
            Err(ValidationError::new(
                "window",
                "window id and dimensions must be positive",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerActionName {
    Activate,
    Launch,
    Click,
    Drag,
    Scroll,
    TypeText,
    PressKeys,
    Close,
    SetClipboard,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerOutput {
    ComputerWindows {
        windows: Vec<ComputerWindowEntry>,
    },
    ComputerScreenshot {
        observation_id: String,
        window: ComputerWindowEntry,
        #[serde(default)]
        media_type: ComputerScreenshotMediaType,
        data_base64: String,
    },
    ComputerAction {
        action: ComputerActionName,
        #[serde(default)]
        window_id: Option<u64>,
        message: String,
    },
    ComputerStop {
        #[serde(default = "default_true")]
        stopped: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerScreenshotMediaType {
    #[default]
    #[serde(rename = "image/png")]
    Png,
}

const fn default_true() -> bool {
    true
}

impl Validate for ComputerOutput {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::ComputerWindows { windows } => windows.iter().try_for_each(Validate::validate),
            Self::ComputerScreenshot {
                observation_id,
                window,
                data_base64,
                ..
            } => {
                length("observation_id", observation_id, 8, 100)?;
                window.validate()?;
                length("data_base64", data_base64, 12, 12_000_000)
            }
            Self::ComputerAction { window_id, .. } => {
                if window_id == &Some(0) {
                    Err(ValidationError::new("window_id", "must be positive"))
                } else {
                    Ok(())
                }
            }
            Self::ComputerStop { stopped, .. } => {
                if *stopped {
                    Ok(())
                } else {
                    Err(ValidationError::new("stopped", "must be true"))
                }
            }
        }
    }
}
