//! The one error type every command returns to the page.

use serde::Serialize;

/// `kind` is `"refused"` (the request is not allowed in the current state, nothing ran)
/// or `"failed"` (it was allowed but could not be done); `message` is English text for the user.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CmdError {
    pub kind: &'static str,
    pub message: String,
}

impl CmdError {
    pub fn refused(message: impl Into<String>) -> Self {
        Self { kind: "refused", message: message.into() }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self { kind: "failed", message: message.into() }
    }
}

pub type CmdResult<T> = Result<T, CmdError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_error_serializes_kind_and_message() {
        assert_eq!(
            serde_json::to_string(&CmdError::refused("No.")).unwrap(),
            r#"{"kind":"refused","message":"No."}"#
        );
    }
}
