use axum::{Json, http::StatusCode, response::IntoResponse};
use iamrust_application::ApplicationError;
use iamrust_domain::DomainError;
use iamrust_protocol::{ApiError, ErrorCode};
use uuid::Uuid;

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    body: ApiError,
}

impl AppError {
    pub fn validation(field: impl Into<String>) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Validation,
            "error.validation",
            Some(field.into()),
            false,
        )
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::AuthenticationRequired,
            "error.authentication_required",
            None,
            false,
        )
    }

    pub fn rate_limited() -> Self {
        Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "error.rate_limited",
            None,
            true,
        )
    }

    pub fn new(
        status: StatusCode,
        code: ErrorCode,
        message_key: impl Into<String>,
        field: Option<String>,
        retryable: bool,
    ) -> Self {
        Self {
            status,
            body: ApiError {
                code,
                message_key: message_key.into(),
                field,
                correlation_id: Uuid::now_v7(),
                retryable,
            },
        }
    }
}

impl From<ApplicationError> for AppError {
    fn from(error: ApplicationError) -> Self {
        match error {
            ApplicationError::Domain(domain) => Self::from(domain),
            ApplicationError::AccountConflict | ApplicationError::Conflict => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
                "error.conflict",
                None,
                false,
            ),
            ApplicationError::InvalidCredentials => Self::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthenticationFailed,
                "error.invalid_credentials",
                None,
                false,
            ),
            ApplicationError::SecondFactorRequired => Self::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthenticationFailed,
                "error.second_factor_required",
                Some("second_factor_code".to_owned()),
                false,
            ),
            ApplicationError::InvalidSecondFactor => Self::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthenticationFailed,
                "error.invalid_second_factor",
                Some("second_factor_code".to_owned()),
                false,
            ),
            ApplicationError::Unauthorized | ApplicationError::SessionExpired => {
                Self::unauthorized()
            }
            ApplicationError::RefreshTokenReuse => Self::new(
                StatusCode::UNAUTHORIZED,
                ErrorCode::AuthenticationFailed,
                "error.refresh_token_reuse",
                None,
                false,
            ),
            ApplicationError::NotFound => Self::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "error.not_found",
                None,
                false,
            ),
            ApplicationError::Storage => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::ServiceUnavailable,
                "error.storage_unavailable",
                None,
                true,
            ),
        }
    }
}

impl From<DomainError> for AppError {
    fn from(error: DomainError) -> Self {
        match error {
            DomainError::Validation { field, .. } => Self::validation(field),
            DomainError::Forbidden | DomainError::SelfTarget => Self::new(
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                "error.forbidden",
                None,
                false,
            ),
            DomainError::NotFound => Self::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "error.not_found",
                None,
                false,
            ),
            DomainError::Conflict | DomainError::InvalidTransition { .. } => Self::new(
                StatusCode::CONFLICT,
                ErrorCode::Conflict,
                "error.conflict",
                None,
                false,
            ),
            DomainError::EmptyMessage | DomainError::MessageTooLarge | DomainError::StaleCursor => {
                Self::validation("message")
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.body)).into_response()
    }
}
