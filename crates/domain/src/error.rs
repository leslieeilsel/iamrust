use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("validation failed for {field}: {reason}")]
    Validation {
        field: &'static str,
        reason: &'static str,
    },
    #[error("operation is not permitted")]
    Forbidden,
    #[error("resource was not found")]
    NotFound,
    #[error("resource already exists")]
    Conflict,
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition {
        from: &'static str,
        to: &'static str,
    },
    #[error("a user cannot target themselves")]
    SelfTarget,
    #[error("the sync cursor moved backwards")]
    StaleCursor,
    #[error("message is too large")]
    MessageTooLarge,
    #[error("message cannot be empty")]
    EmptyMessage,
}
