//! Stored rosters.
//!
//! A roster arrives with a `source` saying what format it is in: `manual`
//! is the canonical [`Roster`] shape; `krooster` and `ak-planner` are
//! those tools' exports, which the Layer 8 adapters in [`ak_data::import`]
//! turn into a [`Roster`]. The stored document keeps the canonical roster,
//! the source it came from, and for an import, the report of what was
//! skipped or changed. `POST /api/v1/rosters/preview` runs the same import
//! without storing anything.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ak_data::import::{self, ImportReport};
use ak_domain::{GameData, Roster};
use ak_store::{Collection, DocumentMeta};

use crate::AppState;
use crate::error::{ApiError, Body, from_value};
use crate::stored::{self, Stored};

/// The format a roster was supplied in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RosterSource {
    /// The canonical shape: `{ operator id: { promotion, mood? } }`.
    #[default]
    Manual,
    /// A Krooster roster (see [`ak_data::import::krooster`]).
    Krooster,
    /// An ak-planner export (see [`ak_data::import::ak_planner`]).
    AkPlanner,
}

impl RosterSource {
    /// The import adapter for this source, if it is not the canonical
    /// shape.
    fn adapter(self) -> Option<import::Source> {
        match self {
            RosterSource::Manual => None,
            RosterSource::Krooster => Some(import::Source::Krooster),
            RosterSource::AkPlanner => Some(import::Source::AkPlanner),
        }
    }
}

/// `POST` and `PUT` body.
#[derive(Deserialize)]
pub struct RosterBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    source: RosterSource,
    roster: Value,
}

/// A stored roster's body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterView {
    /// The canonical roster.
    pub roster: Roster,
    /// What it was imported from.
    #[serde(default)]
    pub source: RosterSource,
    /// For an import, what was skipped or changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportReport>,
}

/// What `POST` and `PUT` answer: the metadata, and the import report.
#[derive(Serialize)]
struct Saved {
    #[serde(flatten)]
    meta: DocumentMeta,
    source: RosterSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    import: Option<ImportReport>,
}

/// Rejects operators the game data does not know.
pub fn check_roster(data: &GameData, roster: &Roster) -> Result<(), ApiError> {
    let unknown: Vec<&str> = roster
        .entries
        .keys()
        .filter(|id| !data.operators.contains_key(id.as_str()))
        .map(|id| id.as_str())
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "unknown operators: {}",
            unknown.join(", ")
        )))
    }
}

/// Turns a supplied roster into the canonical one. Imports skip operators
/// the game data lacks (and say so in the report); a canonical roster must
/// not name any.
fn read(data: &GameData, source: RosterSource, raw: Value) -> Result<RosterView, ApiError> {
    let view = match source.adapter() {
        None => {
            let roster: Roster = from_value(raw)?;
            check_roster(data, &roster)?;
            RosterView {
                roster,
                source,
                import: None,
            }
        }
        Some(adapter) => {
            let imported = import::import(adapter, data, &raw).map_err(ApiError::bad_request)?;
            RosterView {
                roster: imported.roster,
                source,
                import: Some(imported.report),
            }
        }
    };
    Ok(view)
}

fn saved(meta: DocumentMeta, view: RosterView) -> Saved {
    Saved {
        meta,
        source: view.source,
        import: view.import,
    }
}

/// `POST /api/v1/rosters`: imports and stores a roster, answering `201`
/// with its metadata and, for an import, the report.
pub async fn create(
    State(s): State<AppState>,
    Body(body): Body<RosterBody>,
) -> Result<Response, ApiError> {
    let view = read(&s.data, body.source, body.roster)?;
    let meta = stored::insert(&s, Collection::Rosters, body.name, &view).await?;
    Ok((StatusCode::CREATED, Json(saved(meta, view))).into_response())
}

/// `POST /api/v1/rosters/preview`: the roster and report an import would
/// store, without storing it.
pub async fn preview(
    State(s): State<AppState>,
    Body(body): Body<RosterBody>,
) -> Result<Response, ApiError> {
    Ok(Json(read(&s.data, body.source, body.roster)?).into_response())
}

/// `GET /api/v1/rosters`.
pub async fn list(State(s): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(stored::listing(&s, Collection::Rosters).await?).into_response())
}

/// `GET /api/v1/rosters/{id}`.
pub async fn get(State(s): State<AppState>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let doc = stored::fetch(&s, Collection::Rosters, id).await?;
    let view: Stored<RosterView> = stored::view(doc)?;
    Ok(Json(view).into_response())
}

/// `PUT /api/v1/rosters/{id}`: replaces a roster, importing it again if
/// it is an export.
pub async fn put(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<RosterBody>,
) -> Result<Response, ApiError> {
    let view = read(&s.data, body.source, body.roster)?;
    let meta = stored::update(&s, Collection::Rosters, id, body.name, &view).await?;
    Ok(Json(saved(meta, view)).into_response())
}

/// `DELETE /api/v1/rosters/{id}`.
pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    stored::remove(&s, Collection::Rosters, id).await
}
