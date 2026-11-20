//! What a route answers when it can't do what was asked
//!
//! A refused or stale write is an answer, not an error: it comes back as its outcome. An error is
//! a request the service can't act on, such as a plan that isn't listed or a body that isn't
//! JSON, or a failure inside the service. Each has a code a client can match on, and a message a
//! person can read.

use serde::{Deserialize, Serialize};

/// The kinds of error a route answers with
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    /// The project named isn't registered (404)
    NotRegistered,
    /// The plan named isn't listed in its project (404)
    NotListed,
    /// What was asked for isn't there, such as a node or a directory's project (404)
    NotFound,
    /// A project or plan name, or an id, breaks the rule names keep to (400)
    BadName,
    /// The request can't be read, or asks for what the service can't keep (400)
    BadRequest,
    /// The service failed (500)
    Internal,
}

impl ErrorCode {
    /// The HTTP status a route answers this kind of error with
    pub fn status(self) -> u16 {
        match self {
            ErrorCode::NotRegistered | ErrorCode::NotListed | ErrorCode::NotFound => 404,
            ErrorCode::BadName | ErrorCode::BadRequest => 400,
            ErrorCode::Internal => 500,
        }
    }
}

/// An error a route answers with
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// What kind of error it is
    pub code: ErrorCode,
    /// What went wrong, for a person to read
    pub message: String,
}

impl ApiError {
    /// An error of a kind, saying what went wrong
    ///
    /// # Arguments
    ///
    /// * `code` - What kind of error it is
    /// * `message` - What went wrong
    pub fn new<T: Into<String>>(code: ErrorCode, message: T) -> ApiError {
        ApiError {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    /// Shows the error's message
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[cfg(test)]
mod tests {
    use super::{ApiError, ErrorCode};
    use serde_json::json;

    /// An error names its code in kebab-case beside its message
    #[test]
    fn error_names_its_code() {
        // a plan that isn't listed
        let error = ApiError::new(ErrorCode::NotListed, "plan `reef/x` is not listed");
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"code": "not-listed", "message": "plan `reef/x` is not listed"})
        );
        assert_eq!(error.code.status(), 404);
    }
}
