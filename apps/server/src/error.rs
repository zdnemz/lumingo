//! How a failed request is answered.
//!
//! Every error is the JSON body `{"error": "<code>", "message": "..."}`. The
//! request guard already answers `{"error": "<code>"}` for its own refusals, so
//! the UI reads one field in both cases. Statuses 401 and 403 belong to the
//! guard alone: the UI treats them as "the server refused this page", so a
//! failure of the application itself never uses them.

use app_core::CoreError;
use app_core::api::{ApiErrorBody, ErrorCode};
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// The HTTP status for an error code.
pub fn status_for(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorCode::Conflict
        | ErrorCode::ReadOnly
        | ErrorCode::Busy
        | ErrorCode::ProviderNotConfigured => StatusCode::CONFLICT,
        ErrorCode::NotAvailable => StatusCode::NOT_IMPLEMENTED,
        ErrorCode::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Storage | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// An error ready to be sent.
#[derive(Debug)]
pub struct ApiError(ApiErrorBody);

impl ApiError {
    /// An unknown route under `/api`.
    pub fn not_found() -> Self {
        Self(ApiErrorBody {
            error: ErrorCode::NotFound,
            message: "there is no such API route".to_owned(),
        })
    }

    /// A request the server could not read: bad JSON, a wrong field, a bad id.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self(ApiErrorBody {
            error: ErrorCode::InvalidInput,
            message: message.into(),
        })
    }

    pub fn body(&self) -> &ApiErrorBody {
        &self.0
    }
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        Self(error.body())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (status_for(self.0.error), Json(self.0)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_status_and_none_borrows_the_guards() {
        let codes = [
            ErrorCode::NotFound,
            ErrorCode::InvalidInput,
            ErrorCode::Conflict,
            ErrorCode::ReadOnly,
            ErrorCode::Busy,
            ErrorCode::NotAvailable,
            ErrorCode::ProviderNotConfigured,
            ErrorCode::ShuttingDown,
            ErrorCode::Storage,
            ErrorCode::Internal,
        ];
        for code in codes {
            let status = status_for(code);
            assert_ne!(status, StatusCode::UNAUTHORIZED, "{code:?}");
            assert_ne!(status, StatusCode::FORBIDDEN, "{code:?}");
            assert!(!status.is_success(), "{code:?}");
        }
        assert_eq!(
            status_for(ErrorCode::NotAvailable),
            StatusCode::NOT_IMPLEMENTED
        );
    }

    #[test]
    fn a_not_available_feature_is_a_typed_501() {
        let error = ApiError::from(CoreError::NotAvailable(app_core::api::Feature::Speech));
        assert_eq!(error.body().error, ErrorCode::NotAvailable);
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
