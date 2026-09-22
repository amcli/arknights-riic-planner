//! Synchronous Layer 4 endpoints: evaluate one instant, or simulate a
//! horizon.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::Value;

use ak_eval::{SimConfig, SimRequest};

use crate::error::{ApiError, Body};
use crate::stored::blocking;
use crate::{AppState, Limits, refs};

/// Rejects a run with more ticks than the server allows.
pub fn check_ticks(config: &SimConfig, limits: &Limits) -> Result<(), ApiError> {
    let ticks = (config.horizon_hours * 60.0 / f64::from(config.tick_minutes.max(1))).ceil();
    if ticks > limits.max_ticks as f64 {
        return Err(ApiError::bad_request(format!(
            "{ticks:.0} ticks ({} h at {} min) is above this server's limit of {}; \
             shorten the horizon or lengthen the tick",
            config.horizon_hours, config.tick_minutes, limits.max_ticks
        )));
    }
    Ok(())
}

/// `POST /api/v1/evaluate`: room stats and morale rates at the starting
/// instant. `base_id` and `roster_id` may stand in for `base` and `roster`.
pub async fn evaluate(
    State(s): State<AppState>,
    Body(body): Body<Value>,
) -> Result<Response, ApiError> {
    let (req, _) = refs::resolve::<SimRequest>(&s, body).await?;
    let data = s.data.clone();
    let snapshot = blocking(move || {
        ak_eval::evaluate(&data, &req.base, &req.assignment, &req.roster, &req.config)
    })
    .await?
    .map_err(ApiError::bad_request)?;
    Ok(Json(snapshot).into_response())
}

/// `POST /api/v1/simulate`: runs the mood-aware simulator over the
/// request's horizon. `base_id` and `roster_id` may stand in for `base` and
/// `roster`.
pub async fn simulate(
    State(s): State<AppState>,
    Body(body): Body<Value>,
) -> Result<Response, ApiError> {
    let (req, _) = refs::resolve::<SimRequest>(&s, body).await?;
    check_ticks(&req.config, &s.limits)?;
    let data = s.data.clone();
    let result = blocking(move || ak_eval::simulate(&data, &req))
        .await?
        .map_err(ApiError::bad_request)?;
    Ok(Json(result).into_response())
}
