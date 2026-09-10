use cdr_core::{Validate, ValidationError, ValidationResult, count, length};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerApp {
    Chrome,
    Notepad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerMouseButton {
    Left,
    Right,
    Middle,
}

fn default_mouse_button() -> ComputerMouseButton {
    ComputerMouseButton::Left
}
fn default_click_count() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerRequest {
    ComputerListWindows,
    ComputerActivate {
        window_id: u64,
    },
    ComputerLaunch {
        app: ComputerApp,
    },
    ComputerScreenshot {
        window_id: u64,
    },
    ComputerClick {
        window_id: u64,
        observation_id: String,
        x: u32,
        y: u32,
        #[serde(default = "default_mouse_button")]
        button: ComputerMouseButton,
        #[serde(default = "default_click_count")]
        click_count: u8,
    },
    ComputerDrag {
        window_id: u64,
        observation_id: String,
        start_x: u32,
        start_y: u32,
        end_x: u32,
        end_y: u32,
    },
    ComputerScroll {
        window_id: u64,
        observation_id: String,
        x: u32,
        y: u32,
        #[serde(default)]
        delta_x: i32,
        #[serde(default)]
        delta_y: i32,
    },
    ComputerTypeText {
        window_id: u64,
        observation_id: String,
        text: String,
    },
    ComputerPressKeys {
        window_id: u64,
        observation_id: String,
        keys: Vec<String>,
    },
    ComputerClose {
        window_id: u64,
        observation_id: String,
    },
    ComputerSetClipboard {
        window_id: u64,
        observation_id: String,
        text: String,
    },
    ComputerStop,
}

impl Validate for ComputerRequest {
    fn validate(&self) -> ValidationResult {
        match self {
            Self::ComputerListWindows | Self::ComputerLaunch { .. } | Self::ComputerStop => Ok(()),
            Self::ComputerActivate { window_id } | Self::ComputerScreenshot { window_id } => {
                positive_window(*window_id)
            }
            Self::ComputerClick {
                window_id,
                observation_id,
                x,
                y,
                click_count,
                ..
            } => {
                observed(*window_id, observation_id)?;
                coordinate("x", *x)?;
                coordinate("y", *y)?;
                if (1..=3).contains(click_count) {
                    Ok(())
                } else {
                    Err(ValidationError::new(
                        "click_count",
                        "must be between 1 and 3",
                    ))
                }
            }
            Self::ComputerDrag {
                window_id,
                observation_id,
                start_x,
                start_y,
                end_x,
                end_y,
            } => {
                observed(*window_id, observation_id)?;
                for (field, value) in [
                    ("start_x", start_x),
                    ("start_y", start_y),
                    ("end_x", end_x),
                    ("end_y", end_y),
                ] {
                    coordinate(field, *value)?;
                }
                Ok(())
            }
            Self::ComputerScroll {
                window_id,
                observation_id,
                x,
                y,
                delta_x,
                delta_y,
            } => {
                observed(*window_id, observation_id)?;
                coordinate("x", *x)?;
                coordinate("y", *y)?;
                if !(-10_000..=10_000).contains(delta_x) || !(-10_000..=10_000).contains(delta_y) {
                    return Err(ValidationError::new(
                        "delta",
                        "must be between -10000 and 10000",
                    ));
                }
                if *delta_x == 0 && *delta_y == 0 {
                    Err(ValidationError::new(
                        "delta",
                        "A non-zero scroll amount is required.",
                    ))
                } else {
                    Ok(())
                }
            }
            Self::ComputerTypeText {
                window_id,
                observation_id,
                text,
            } => {
                observed(*window_id, observation_id)?;
                length("text", text, 1, 4_096)
            }
            Self::ComputerPressKeys {
                window_id,
                observation_id,
                keys,
            } => {
                observed(*window_id, observation_id)?;
                count("keys", keys.len(), 1, 4)
            }
            Self::ComputerClose {
                window_id,
                observation_id,
            } => observed(*window_id, observation_id),
            Self::ComputerSetClipboard {
                window_id,
                observation_id,
                text,
            } => {
                observed(*window_id, observation_id)?;
                length("text", text, 0, 100_000)
            }
        }
    }
}

fn positive_window(value: u64) -> ValidationResult {
    if value > 0 {
        Ok(())
    } else {
        Err(ValidationError::new("window_id", "must be positive"))
    }
}
fn observed(window_id: u64, observation_id: &str) -> ValidationResult {
    positive_window(window_id)?;
    length("observation_id", observation_id, 8, 100)
}
fn coordinate(field: &'static str, value: u32) -> ValidationResult {
    if value <= 100_000 {
        Ok(())
    } else {
        Err(ValidationError::new(field, "must not exceed 100000"))
    }
}
