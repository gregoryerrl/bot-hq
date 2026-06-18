//! Typed error surface for Tauri commands.
//!
//! The frontend `useInvoke` hook matches on `AppError.kind` to decide
//! between toast (mutations), error boundary (queries), or silent retry
//! (network). Each variant carries a human-readable message.

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    /// Bad input. Frontend should highlight the offending field if it knows
    /// which one. Otherwise a toast.
    Validation(String),
    /// Resource doesn't exist (404-ish). Frontend can redirect or show empty
    /// state.
    NotFound(String),
    /// Authentication / authorization failure (e.g., trying to call a HANDS-
    /// only tool from EYES, or a plugin missing a capability grant).
    Unauthorized(String),
    /// Unexpected error not classified elsewhere. Toast with the message.
    Internal(String),
    /// Sqlite write/read failure. Toast + log + offer retry.
    DbError(String),
    /// Tauri capability check denied a plugin's request.
    CapabilityDenied(String),
    /// Resource already exists (409-ish). Used by plugin install when the
    /// target id is already on disk; frontend can offer "reinstall" UX.
    Conflict(String),
}

impl AppError {
    pub fn internal(msg: impl Into<String>) -> Self {
        AppError::Internal(msg.into())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        // `{:#}` renders the FULL anyhow chain (every context layer + the root
        // cause), not just the outermost context that plain `Display`
        // (`to_string()`) yields. Without it, a failed subprocess spawn surfaces
        // only "spawning claude-code for agent …" and hides the real OS error.
        AppError::Internal(format!("{e:#}"))
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Validation(m) => write!(f, "validation: {m}"),
            AppError::NotFound(m) => write!(f, "not_found: {m}"),
            AppError::Unauthorized(m) => write!(f, "unauthorized: {m}"),
            AppError::Internal(m) => write!(f, "internal: {m}"),
            AppError::DbError(m) => write!(f, "db_error: {m}"),
            AppError::CapabilityDenied(m) => write!(f, "capability_denied: {m}"),
            AppError::Conflict(m) => write!(f, "conflict: {m}"),
        }
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let err = AppError::NotFound("session sx".to_string());
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["kind"], "NotFound");
        assert_eq!(v["message"], "session sx");
    }

    #[test]
    fn anyhow_converts_to_internal() {
        let err = AppError::from(anyhow::anyhow!("boom"));
        assert!(matches!(err, AppError::Internal(_)));
    }

    #[test]
    fn anyhow_internal_keeps_full_chain() {
        // A failed spawn wraps the OS error in a context layer; the surfaced
        // message must include BOTH the context and the root cause, not just the
        // outermost context (which is all plain `to_string()` would give).
        let root = std::io::Error::new(std::io::ErrorKind::NotFound, "os error 206");
        let chained =
            anyhow::Error::new(root).context("spawning claude-code for agent rain");
        let AppError::Internal(msg) = AppError::from(chained) else {
            panic!("expected Internal");
        };
        assert!(msg.contains("spawning claude-code"), "missing context: {msg}");
        assert!(msg.contains("os error 206"), "missing root cause: {msg}");
    }
}
