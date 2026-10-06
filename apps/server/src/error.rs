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
        | ErrorCode::LicenceNotAccepted
        | ErrorCode::ProviderNotConfigured => StatusCode::CONFLICT,
        ErrorCode::NotAvailable => StatusCode::NOT_IMPLEMENTED,
        ErrorCode::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Storage | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// An error ready to be sent.
#[derive(Debug)]
pub struct ApiError {
    body: ApiErrorBody,
    /// Overrides the status of the code, for the one case that has no code of
    /// its own.
    status: Option<StatusCode>,
}

impl ApiError {
    /// An unknown route under `/api`.
    pub fn not_found() -> Self {
        Self::coded(ErrorCode::NotFound, "there is no such API route")
    }

    fn coded(error: ErrorCode, message: &str) -> Self {
        Self {
            body: ApiErrorBody {
                error,
                message: message.to_owned(),
                feature: None,
            },
            status: None,
        }
    }

    /// A body above [`crate::MAX_BODY_BYTES`]: HTTP 413 with the usual body.
    pub fn payload_too_large() -> Self {
        Self {
            status: Some(StatusCode::PAYLOAD_TOO_LARGE),
            ..Self::coded(ErrorCode::InvalidInput, "the request body is too large")
        }
    }

    /// A request the server could not read: bad JSON, a wrong field, a bad id.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::coded(ErrorCode::InvalidInput, &message.into())
    }

    pub fn body(&self) -> &ApiErrorBody {
        &self.body
    }
}

impl From<CoreError> for ApiError {
    fn from(error: CoreError) -> Self {
        Self {
            body: error.body(),
            status: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status.unwrap_or_else(|| status_for(self.body.error));
        (status, Json(self.body)).into_response()
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
            ErrorCode::LicenceNotAccepted,
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
    fn a_not_available_feature_is_a_typed_501_that_names_the_feature() {
        let error = ApiError::from(CoreError::NotAvailable(app_core::api::Feature::Speech));
        assert_eq!(error.body().error, ErrorCode::NotAvailable);
        assert_eq!(error.body().feature, Some(app_core::api::Feature::Speech));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn a_missing_part_with_a_reason_is_a_501_with_the_reason_and_a_licence_refusal_a_409() {
        let missing = ApiError::from(CoreError::unavailable(None, "the placement bank is missing"));
        assert_eq!(missing.body().error, ErrorCode::NotAvailable);
        assert_eq!(missing.body().message, "the placement bank is missing");
        assert_eq!(missing.into_response().status(), StatusCode::NOT_IMPLEMENTED);
        let licence = ApiError::from(CoreError::LicenceNotAccepted("accept it first".to_owned()));
        assert_eq!(licence.body().error, ErrorCode::LicenceNotAccepted);
        assert_eq!(licence.into_response().status(), StatusCode::CONFLICT);
    }
}
