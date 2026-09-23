//! Read-only game data: provenance, operators, skills, room kinds and
//! Factory formulas.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use ak_data::stats::DataStats;
use ak_domain::{
    BaseLayout, BaseSkill, DataVersion, Facility, GameConstants, ManufactureFormula, Operator,
    PowerId, Profession, Rarity,
};

use crate::AppState;
use crate::error::ApiError;

#[derive(Serialize)]
struct VersionResponse<'a> {
    #[serde(flatten)]
    version: &'a DataVersion,
    stats: &'a DataStats,
    operators_skipped: usize,
}

/// `GET /api/v1/gamedata/version`: where the data came from and how much
/// of it the parser models.
pub async fn version(State(s): State<AppState>) -> Response {
    Json(VersionResponse {
        version: &s.data.version,
        stats: &s.stats,
        operators_skipped: s.operators_skipped,
    })
    .into_response()
}

#[derive(Serialize)]
struct OperatorSummary<'a> {
    id: &'a str,
    name: &'a str,
    rarity: Rarity,
    stars: u8,
    profession: Profession,
    sub_profession: &'a str,
    nation: Option<&'a PowerId>,
    group: Option<&'a PowerId>,
    team: Option<&'a PowerId>,
    max_levels: &'a [u32],
}

impl<'a> From<&'a Operator> for OperatorSummary<'a> {
    fn from(op: &'a Operator) -> Self {
        OperatorSummary {
            id: op.id.as_str(),
            name: &op.name,
            rarity: op.rarity,
            stars: op.rarity.stars(),
            profession: op.profession,
            sub_profession: op.sub_profession.as_str(),
            nation: op.nation.as_ref(),
            group: op.group.as_ref(),
            team: op.team.as_ref(),
            max_levels: &op.max_levels,
        }
    }
}

/// `GET /api/v1/gamedata/operators`: every operator, summarised.
pub async fn list_operators(State(s): State<AppState>) -> Response {
    let list: Vec<OperatorSummary<'_>> = s.data.operators.values().map(Into::into).collect();
    Json(list).into_response()
}

/// `GET /api/v1/gamedata/operators/{id}`.
pub async fn get_operator(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    match s.data.operator(&id) {
        Some(op) => Ok(Json::<&Operator>(op).into_response()),
        None => Err(ApiError::not_found(format!("no operator {id:?}"))),
    }
}

/// `GET /api/v1/gamedata/skills/{id}`, with parsed mechanics.
pub async fn get_skill(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    match s.data.skill(&id) {
        Some(skill) => Ok(Json::<&BaseSkill>(skill).into_response()),
        None => Err(ApiError::not_found(format!("no skill {id:?}"))),
    }
}

/// `GET /api/v1/gamedata/facilities`: every room kind with its per-level
/// capacity, power and build cost, for building a base.
pub async fn list_facilities(State(s): State<AppState>) -> Response {
    let list: Vec<&Facility> = s.data.facilities.values().collect();
    Json(list).into_response()
}

/// `GET /api/v1/gamedata/formulas`: every Factory formula, for choosing
/// what a Factory makes.
pub async fn list_formulas(State(s): State<AppState>) -> Response {
    let list: Vec<&ManufactureFormula> = s.data.manufacture_formulas.values().collect();
    Json(list).into_response()
}

/// `GET /api/v1/gamedata/layout`: the base's slots with their grid
/// positions, for drawing a base.
pub async fn layout(State(s): State<AppState>) -> Response {
    Json::<&BaseLayout>(&s.data.layout).into_response()
}

/// `GET /api/v1/gamedata/constants`: global tuning values, such as the
/// Dormitory ambience limit.
pub async fn constants(State(s): State<AppState>) -> Response {
    Json::<&GameConstants>(&s.data.constants).into_response()
}
