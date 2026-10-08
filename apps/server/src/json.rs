//! Request extractors that answer in the API's own error format.
//!
//! axum's own rejections are plain text, which the UI cannot read as an error
//! code. These wrappers turn them into the JSON error body.

use axum::body::Bytes;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// A JSON request body, or an `invalid_input` error that does not echo the body.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Self(value)),
            // The body can hold a key, so the message names the kind of problem
            // and never the text.
            Err(JsonRejection::JsonDataError(_)) => Err(ApiError::invalid(
                "the request body does not have the expected fields",
            )),
            Err(JsonRejection::JsonSyntaxError(_)) => {
                Err(ApiError::invalid("the request body is not valid JSON"))
            }
            Err(JsonRejection::BytesRejection(rejection))
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE =>
            {
                Err(ApiError::payload_too_large())
            }
            Err(JsonRejection::MissingJsonContentType(_)) => {
                Err(ApiError::invalid("the request must be application/json"))
            }
            Err(_) => Err(ApiError::invalid("the request body could not be read")),
        }
    }
}

/// A JSON request body that may be left out: an empty body is `T::default()`.
/// A body that is there is read like [`ApiJson`], with the same size limit.
pub struct OptionalJson<T>(pub T);

impl<S, T> FromRequest<S> for OptionalJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Default,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = Bytes::from_request(req, state).await.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::payload_too_large()
            } else {
                ApiError::invalid("the request body could not be read")
            }
        })?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(Self(T::default()));
        }
        serde_json::from_slice(&bytes)
            .map(Self)
            .map_err(|_| ApiError::invalid("the request body does not have the expected fields"))
    }
}

/// Reads a turn number from a path segment.
pub fn parse_turn(raw: &str) -> Result<u64, ApiError> {
    raw.parse::<u64>()
        .map_err(|_| ApiError::invalid("the turn in the address is not a number"))
}

/// Reads a numeric id from a path segment.
pub fn parse_id(raw: &str) -> Result<i64, ApiError> {
    raw.parse::<i64>()
        .ok()
        .filter(|id| *id >= 0)
        .ok_or_else(|| ApiError::invalid("the id in the address is not a number"))
}

#[cfg(test)]
mod tests {
    use super::{parse_id, parse_turn};

    #[test]
    fn turns_are_non_negative_integers() {
        assert_eq!(parse_turn("7").ok(), Some(7));
        for bad in ["", "-1", "1.5", "x", "18446744073709551616"] {
            assert!(parse_turn(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn ids_are_non_negative_integers() {
        assert_eq!(parse_id("42").ok(), Some(42));
        assert_eq!(parse_id("0").ok(), Some(0));
        for bad in ["", "-1", "1.5", "abc", "99999999999999999999", "1 "] {
            assert!(parse_id(bad).is_err(), "{bad:?}");
        }
    }
}
