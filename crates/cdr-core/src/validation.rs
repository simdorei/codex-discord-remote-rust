use thiserror::Error;

pub type ValidationResult<T = ()> = Result<T, ValidationError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{field}: {message}")]
pub struct ValidationError {
    pub field: &'static str,
    pub message: String,
}

impl ValidationError {
    #[must_use]
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

pub trait Validate {
    fn validate(&self) -> ValidationResult;
}

pub fn length(field: &'static str, value: &str, min: usize, max: usize) -> ValidationResult {
    let count = value.chars().count();
    if (min..=max).contains(&count) {
        Ok(())
    } else {
        Err(ValidationError::new(
            field,
            format!("length must be between {min} and {max}"),
        ))
    }
}

pub fn count(field: &'static str, actual: usize, min: usize, max: usize) -> ValidationResult {
    if (min..=max).contains(&actual) {
        Ok(())
    } else {
        Err(ValidationError::new(
            field,
            format!("item count must be between {min} and {max}"),
        ))
    }
}
