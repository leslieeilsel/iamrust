use std::sync::LazyLock;

use regex::Regex;

use crate::DomainError;

pub const MAX_MESSAGE_CHARS: usize = 10_000;
const MIN_PASSWORD_CHARS: usize = 10;
const MAX_PASSWORD_CHARS: usize = 128;
const MIN_USERNAME_CHARS: usize = 3;
const MAX_USERNAME_CHARS: usize = 32;

static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
        .expect("email regex is valid")
});

static USERNAME_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_]+$").expect("username regex is valid"));

pub fn validate_email(value: &str) -> Result<(), DomainError> {
    if value.len() > 254 || !EMAIL_RE.is_match(value) {
        return Err(DomainError::Validation {
            field: "email",
            reason: "invalid_email",
        });
    }
    Ok(())
}

pub fn validate_username(value: &str) -> Result<(), DomainError> {
    let count = value.chars().count();
    if !(MIN_USERNAME_CHARS..=MAX_USERNAME_CHARS).contains(&count) || !USERNAME_RE.is_match(value) {
        return Err(DomainError::Validation {
            field: "username",
            reason: "invalid_username",
        });
    }
    Ok(())
}

pub fn validate_password(value: &str) -> Result<(), DomainError> {
    let count = value.chars().count();
    let strong_enough = value.chars().any(char::is_uppercase)
        && value.chars().any(char::is_lowercase)
        && value.chars().any(|character| character.is_ascii_digit());
    if !(MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&count) || !strong_enough {
        return Err(DomainError::Validation {
            field: "password",
            reason: "weak_password",
        });
    }
    Ok(())
}

pub fn validate_message_text(value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        return Err(DomainError::EmptyMessage);
    }
    if value.chars().count() > MAX_MESSAGE_CHARS {
        return Err(DomainError::MessageTooLarge);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_account_fields() {
        assert!(validate_email("hello@example.com").is_ok());
        assert!(validate_email("missing-at.example.com").is_err());
        assert!(validate_username("rust_user").is_ok());
        assert!(validate_username("no spaces").is_err());
        assert!(validate_password("StrongPass1").is_ok());
        assert!(validate_password("weak").is_err());
    }

    #[test]
    fn rejects_blank_and_oversized_messages() {
        assert_eq!(
            validate_message_text(" \n "),
            Err(DomainError::EmptyMessage)
        );
        let oversized = "文".repeat(MAX_MESSAGE_CHARS + 1);
        assert_eq!(
            validate_message_text(&oversized),
            Err(DomainError::MessageTooLarge)
        );
    }
}
