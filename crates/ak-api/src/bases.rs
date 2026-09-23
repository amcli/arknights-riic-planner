//! Stored bases, validated against the game data on the way in.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use ak_domain::BaseConfig;
use ak_store::Collection;

use crate::AppState;
use crate::error::{ApiError, Body};
use crate::stored::{self, Stored};

/// `POST` and `PUT` body.
#[derive(Deserialize)]
pub struct BaseBody {
    #[serde(default)]
    name: Option<String>,
    base: BaseConfig,
}

/// A stored base's body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseView {
    /// The rooms.
    pub base: BaseConfig,
}

/// `POST /api/v1/bases`.
pub async fn create(
    State(s): State<AppState>,
    Body(body): Body<BaseBody>,
) -> Result<Response, ApiError> {
    body.base.validate(&s.data).map_err(ApiError::bad_request)?;
    let view = BaseView { base: body.base };
    stored::create(&s, Collection::Bases, body.name, view).await
}

/// `GET /api/v1/bases`.
pub async fn list(State(s): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(stored::listing(&s, Collection::Bases).await?).into_response())
}

/// `GET /api/v1/bases/{id}`.
pub async fn get(State(s): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let doc = stored::fetch(&s, Collection::Bases, id).await?;
    let view: Stored<BaseView> = stored::view(doc)?;
    Ok(Json(view).into_response())
}

/// `PUT /api/v1/bases/{id}`.
pub async fn put(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<BaseBody>,
) -> Result<Response, ApiError> {
    body.base.validate(&s.data).map_err(ApiError::bad_request)?;
    let view = BaseView { base: body.base };
    stored::replace(&s, Collection::Bases, id, body.name, view).await
}

/// `DELETE /api/v1/bases/{id}`.
pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    stored::remove(&s, Collection::Bases, id).await
}
