//! Stored rosters.
//!
//! A roster arrives with a `source` saying what format it is in. `manual`
//! is the canonical [`Roster`] shape; the other sources are other tools'
//! exports, which the Layer 8 adapters turn into a [`Roster`]. The stored
//! document keeps the canonical roster and the source it came from.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ak_domain::{GameData, Roster};
use ak_store::Collection;

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
    /// A Krooster export.
    Krooster,
    /// An ak-planner export.
    AkPlanner,
}

impl RosterSource {
    /// The name used in requests.
    pub fn as_str(self) -> &'static str {
        match self {
            RosterSource::Manual => "manual",
            RosterSource::Krooster => "krooster",
            RosterSource::AkPlanner => "ak-planner",
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

/// Turns a supplied roster into the canonical one.
fn import(data: &GameData, source: RosterSource, raw: Value) -> Result<Roster, ApiError> {
    let roster: Roster = match source {
        RosterSource::Manual => from_value(raw)?,
        other => {
            return Err(ApiError::new(
                StatusCode::NOT_IMPLEMENTED,
                format!("importing a {} roster is not supported yet", other.as_str()),
            ));
        }
    };
    check_roster(data, &roster)?;
    Ok(roster)
}

/// `POST /api/v1/rosters`.
pub async fn create(
    State(s): State<AppState>,
    Body(body): Body<RosterBody>,
) -> Result<Response, ApiError> {
    let roster = import(&s.data, body.source, body.roster)?;
    let view = RosterView {
        roster,
        source: body.source,
    };
    stored::create(&s, Collection::Rosters, body.name, view).await
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

/// `PUT /api/v1/rosters/{id}`.
pub async fn put(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Body(body): Body<RosterBody>,
) -> Result<Response, ApiError> {
    let roster = import(&s.data, body.source, body.roster)?;
    let view = RosterView {
        roster,
        source: body.source,
    };
    stored::replace(&s, Collection::Rosters, id, body.name, view).await
}

/// `DELETE /api/v1/rosters/{id}`.
pub async fn delete(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    stored::remove(&s, Collection::Rosters, id).await
}
