//! Helpers shared by the document collections: create, replace, fetch,
//! list and remove, all run off the async executor.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use ak_store::{Collection, Document, DocumentMeta, StoreError};

use crate::AppState;
use crate::error::ApiError;

/// A stored document's metadata followed by its body's fields.
#[derive(Debug, Serialize, Deserialize)]
pub struct Stored<T> {
    /// Id, name, timestamps, schema version.
    #[serde(flatten)]
    pub meta: DocumentMeta,
    /// The body.
    #[serde(flatten)]
    pub body: T,
}

/// Runs file or CPU work off the async executor.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// Stores a new document and answers `201` with its metadata.
pub async fn create(
    s: &AppState,
    collection: Collection,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let body = serde_json::to_value(body).map_err(ApiError::internal)?;
    let doc = Document::new(name, body);
    let meta = doc.meta();
    let store = s.store.clone();
    blocking(move || store.put(collection, &doc)).await??;
    Ok((StatusCode::CREATED, Json(meta)).into_response())
}

/// Replaces a document's body (and its name, when one is given).
pub async fn replace(
    s: &AppState,
    collection: Collection,
    id: String,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let body = serde_json::to_value(body).map_err(ApiError::internal)?;
    let store = s.store.clone();
    let key = id.clone();
    let meta = blocking(move || -> Result<Option<DocumentMeta>, StoreError> {
        let Some(mut doc) = store.get(collection, &key)? else {
            return Ok(None);
        };
        if name.is_some() {
            doc.name = name;
        }
        doc.body = body;
        doc.touch();
        store.put(collection, &doc)?;
        Ok(Some(doc.meta()))
    })
    .await??;
    match meta {
        Some(meta) => Ok(Json(meta).into_response()),
        None => Err(missing(collection, &id)),
    }
}

/// Reads a document, if it exists.
pub async fn find(
    s: &AppState,
    collection: Collection,
    id: String,
) -> Result<Option<Document>, ApiError> {
    let store = s.store.clone();
    Ok(blocking(move || store.get(collection, &id)).await??)
}

/// Reads a document, or answers `404`.
pub async fn fetch(s: &AppState, collection: Collection, id: String) -> Result<Document, ApiError> {
    let doc = find(s, collection, id.clone()).await?;
    doc.ok_or_else(|| missing(collection, &id))
}

/// Lists a collection, newest first.
pub async fn listing(s: &AppState, collection: Collection) -> Result<Vec<DocumentMeta>, ApiError> {
    let store = s.store.clone();
    Ok(blocking(move || store.list(collection)).await??)
}

/// Deletes a document and answers `204`, or `404` if it was not there.
pub async fn remove(
    s: &AppState,
    collection: Collection,
    id: String,
) -> Result<Response, ApiError> {
    let store = s.store.clone();
    let key = id.clone();
    let removed = blocking(move || store.delete(collection, &key)).await??;
    if removed {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(missing(collection, &id))
    }
}

/// A document with its body read as `T`.
pub fn view<T: DeserializeOwned>(doc: Document) -> Result<Stored<T>, ApiError> {
    let meta = doc.meta();
    let body: T = serde_json::from_value(doc.body).map_err(|e| {
        ApiError::internal(format!("stored document {} is unreadable: {e}", meta.id))
    })?;
    Ok(Stored { meta, body })
}

/// The `404` for a missing document.
pub fn missing(collection: Collection, id: &str) -> ApiError {
    let kind = match collection {
        Collection::Rosters => "roster",
        Collection::Bases => "base",
        Collection::Solves => "solve",
    };
    ApiError::not_found(format!("no {kind} with id {id}"))
}
