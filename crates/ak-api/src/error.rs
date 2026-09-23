//! Error responses, and the JSON body extractor that produces them.
//!
//! Every error the API returns has a JSON body `{ "error": message }`,
//! including malformed bodies, unknown routes and wrong methods. Anything
//! the client got wrong in a body is `400`; a missing `Content-Type` is
//! `415` and an oversized body `413`, as axum decides.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::{Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use ak_store::StoreError;

/// An error response: a status and a readable message.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status.
    pub status: StatusCode,
    /// What went wrong, for a person.
    pub message: String,
}

impl ApiError {
    /// An error with the given status.
    pub fn new(status: StatusCode, message: impl ToString) -> Self {
        ApiError {
            status,
            message: message.to_string(),
        }
    }

    /// `400`: the request is wrong.
    pub fn bad_request(message: impl ToString) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    /// `404`: no such thing.
    pub fn not_found(message: impl ToString) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    /// `409`: the thing is not in a state that allows this.
    pub fn conflict(message: impl ToString) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    /// `500`: our fault.
    pub fn internal(message: impl ToString) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::BadId(_) => Self::bad_request(e),
            other => Self::internal(other),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        Self::internal(e)
    }
}

/// A JSON request body whose rejections are [`ApiError`]s, so that a
/// malformed body gets the same `{ "error": … }` shape as everything else.
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Body(value)),
            Err(rejection) => Err(from_rejection(&rejection)),
        }
    }
}

fn from_rejection(rejection: &JsonRejection) -> ApiError {
    let status = match rejection.status() {
        // Well-formed JSON of the wrong shape is still a bad request.
        StatusCode::UNPROCESSABLE_ENTITY => StatusCode::BAD_REQUEST,
        s => s,
    };
    ApiError::new(status, rejection.body_text())
}

/// Deserialises a JSON value, naming the path to whatever did not fit
/// (`base.rooms[2].kind: unknown variant …`).
pub fn from_value<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, ApiError> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e.path().to_string();
        if path == "." {
            ApiError::bad_request(e.into_inner())
        } else {
            ApiError::bad_request(format!("{path}: {}", e.into_inner()))
        }
    })
}

/// Fallback for unknown routes.
pub async fn no_route(method: Method, uri: Uri) -> ApiError {
    ApiError::not_found(format!("no route {method} {}", uri.path()))
}

/// Fallback for a known route called with the wrong method.
pub async fn wrong_method(method: Method, uri: Uri) -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        format!("{} does not accept {method}", uri.path()),
    )
}
