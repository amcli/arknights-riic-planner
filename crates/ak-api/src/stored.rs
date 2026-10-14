//! Helpers shared by the document collections: create, replace, fetch,
//! list and remove, each on behalf of one account (`owner`). Another
//! account's document is answered as missing.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use uuid::Uuid;

use ak_store::{Collection, Document, DocumentMeta};

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

/// Runs blocking or CPU-heavy work off the async executor.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    Ok(tokio::task::spawn_blocking(f).await?)
}

/// Runs `work` to the end even if the request is dropped (the client
/// disconnects) part way through, for changes that must not stop halfway.
pub async fn to_completion<T: Send + 'static>(
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, ApiError> {
    Ok(tokio::spawn(work).await?)
}

/// The id a path or request names, or `400` if it is not one the store
/// could have issued. Only the spelling the store issues (lower-case,
/// hyphenated) is accepted, so each document has one id.
pub fn parse_id(id: &str) -> Result<Uuid, ApiError> {
    Uuid::try_parse(id)
        .ok()
        .filter(|uuid| uuid.hyphenated().to_string() == id)
        .ok_or_else(|| ApiError::bad_request(format!("invalid document id {id:?}")))
}

/// Stores a new document and answers `201` with its metadata.
pub async fn create(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let meta = insert(s, owner, collection, name, body).await?;
    Ok((StatusCode::CREATED, Json(meta)).into_response())
}

/// Stores a new document and returns its metadata.
pub async fn insert(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    name: Option<String>,
    body: impl Serialize,
) -> Result<DocumentMeta, ApiError> {
    let body = serde_json::to_value(body).map_err(ApiError::internal)?;
    let doc = Document::new(owner, name, body);
    s.store.insert(collection, &doc).await?;
    Ok(doc.meta())
}

/// Replaces a document's body (and its name, when one is given) and
/// answers with its metadata.
pub async fn replace(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    id: String,
    name: Option<String>,
    body: impl Serialize,
) -> Result<Response, ApiError> {
    let meta = update(s, owner, collection, id, name, body).await?;
    Ok(Json(meta).into_response())
}

/// Replaces a document's body (and its name, when one is given) and
/// returns its metadata, or `404`.
pub async fn update(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    id: String,
    name: Option<String>,
    body: impl Serialize,
) -> Result<DocumentMeta, ApiError> {
    let body = serde_json::to_value(body).map_err(ApiError::internal)?;
    let Some(mut doc) = find(s, owner, collection, id.clone()).await? else {
        return Err(missing(collection, &id));
    };
    if name.is_some() {
        doc.name = name;
    }
    doc.body = body;
    doc.touch();
    if s.store.update(collection, &doc).await? {
        Ok(doc.meta())
    } else {
        Err(missing(collection, &id))
    }
}

/// Reads a document, if it exists.
pub async fn find(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    id: String,
) -> Result<Option<Document>, ApiError> {
    Ok(s.store.get(owner, collection, parse_id(&id)?).await?)
}

/// Reads a document, or answers `404`.
pub async fn fetch(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    id: String,
) -> Result<Document, ApiError> {
    let doc = find(s, owner, collection, id.clone()).await?;
    doc.ok_or_else(|| missing(collection, &id))
}

/// Lists a collection, newest first.
pub async fn listing(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
) -> Result<Vec<DocumentMeta>, ApiError> {
    Ok(s.store.list(owner, collection).await?)
}

/// Deletes a document and answers `204`, or `404` if it was not there.
pub async fn remove(
    s: &AppState,
    owner: Uuid,
    collection: Collection,
    id: String,
) -> Result<Response, ApiError> {
    if s.store.delete(owner, collection, parse_id(&id)?).await? {
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
