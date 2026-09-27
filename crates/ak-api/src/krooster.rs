//! Rosters fetched from Krooster by username.
//!
//! Krooster's public profile route, `/api/u/{username}`, answers with a
//! user's account details, support units and roster (see
//! [`ak_data::import::krooster`]). It sends no CORS headers, so a page on
//! another origin cannot read it; `GET /api/v1/import/krooster/{username}`
//! fetches it here instead and answers with the roster alone,
//! `{ data: { roster } }`, ready to store with source `krooster`. The
//! account details and supports are dropped as soon as they arrive.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use ureq::tls::{TlsConfig, TlsProvider};

use crate::AppState;
use crate::error::ApiError;
use crate::stored::blocking;

/// Where Krooster lives; the bare `krooster.com` redirects here.
pub const KROOSTER_URL: &str = "https://www.krooster.com";

/// Longest a fetch may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Largest answer read. A full roster is about 100 KB.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Longest Krooster username: its settings page allows 31 characters of
/// letters, digits, `_` and `-`, and its generated names stay within 32.
const MAX_USERNAME: usize = 32;

/// Reads Krooster's public profiles.
#[derive(Debug)]
pub struct Krooster {
    base: String,
    agent: ureq::Agent,
}

impl Default for Krooster {
    fn default() -> Self {
        Self::at(KROOSTER_URL)
    }
}

impl Krooster {
    /// A client for the Krooster at `base`: [`KROOSTER_URL`], or a local
    /// stand-in in tests.
    pub fn at(base: &str) -> Self {
        let config = ureq::Agent::config_builder()
            // The workspace builds ureq with native-tls only, and ureq's
            // default provider is rustls: without this, https panics.
            .tls_config(
                TlsConfig::builder()
                    .provider(TlsProvider::NativeTls)
                    .build(),
            )
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!(
                "arknights-riic-planner/",
                env!("CARGO_PKG_VERSION")
            ))
            .build();
        Krooster {
            base: base.trim_end_matches('/').to_owned(),
            agent: config.into(),
        }
    }

    /// The roster in `username`'s public profile, as
    /// `{ "data": { "roster": … } }`. Blocks until Krooster answers or the
    /// timeout passes.
    pub fn roster(&self, username: &str) -> Result<Value, ApiError> {
        let name = check_username(username)?;
        // Krooster looks usernames up in lower case.
        let url = format!("{}/api/u/{}", self.base, name.to_ascii_lowercase());
        let mut response = match self.agent.get(&url).call() {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => {
                return Err(ApiError::not_found(format!(
                    "no Krooster user named {name:?}"
                )));
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(ApiError::bad_gateway(format!(
                    "Krooster answered {code} when asked for {name:?}"
                )));
            }
            Err(e) => {
                return Err(ApiError::bad_gateway(format!(
                    "could not reach Krooster: {e}"
                )));
            }
        };
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| ApiError::bad_gateway(format!("could not read Krooster's answer: {e}")))?;
        profile_roster(&body).ok_or_else(|| {
            ApiError::bad_gateway(format!(
                "Krooster's answer for {name:?} is not a Krooster profile"
            ))
        })
    }
}

/// `raw`, trimmed, if Krooster could have it as a username. Checked before
/// it goes into a URL, so it can only name a profile.
fn check_username(raw: &str) -> Result<&str, ApiError> {
    let name = raw.trim();
    let valid = !name.is_empty()
        && name.len() <= MAX_USERNAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if valid {
        Ok(name)
    } else {
        Err(ApiError::bad_request(format!(
            "{name:?} is not a Krooster username: those are 1 to {MAX_USERNAME} \
             letters, digits, `_` or `-`"
        )))
    }
}

/// `{ data: { roster } }` out of a profile, without its account details
/// and supports.
fn profile_roster(body: &[u8]) -> Option<Value> {
    let mut profile: Value = serde_json::from_slice(body).ok()?;
    let roster = profile.get_mut("data")?.get_mut("roster")?.take();
    roster
        .is_object()
        .then(|| json!({ "data": { "roster": roster } }))
}

/// `GET /api/v1/import/krooster/{username}`: the roster in a Krooster
/// user's public profile.
pub async fn roster(
    State(s): State<AppState>,
    Path(username): Path<String>,
) -> Result<Response, ApiError> {
    let krooster = s.krooster.clone();
    let roster = blocking(move || krooster.roster(&username)).await??;
    Ok(Json(roster).into_response())
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn https_goes_through_native_tls() {
        let krooster = Krooster::default();
        assert_eq!(
            krooster.agent.config().tls_config().provider(),
            TlsProvider::NativeTls
        );
        assert_eq!(krooster.base, KROOSTER_URL);
    }

    #[test]
    fn usernames_are_checked_like_krooster_does() {
        assert_eq!(
            check_username(" example-doctor ").unwrap(),
            "example-doctor"
        );
        assert!(check_username("Doc_2").is_ok());
        assert!(check_username(&"x".repeat(MAX_USERNAME)).is_ok());
        for bad in [
            "",
            "  ",
            "no.dots",
            "a/b",
            "../x",
            "a b",
            "é",
            &"x".repeat(33),
        ] {
            let err = check_username(bad).unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{bad:?}");
        }
    }

    #[test]
    fn only_the_roster_is_kept() {
        let profile = json!({
            "data": {
                "account": { "username": "example-doctor", "discordcode": "doctor#0001" },
                "supports": [{ "op_id": "char_103_angel", "slot": 0 }],
                "roster": { "char_103_angel": { "op_id": "char_103_angel", "elite": 2 } },
            }
        });
        assert_eq!(
            profile_roster(profile.to_string().as_bytes()),
            Some(json!({ "data": { "roster": profile["data"]["roster"] } }))
        );
        for other in [
            "User not found",
            "{}",
            r#"{ "data": {} }"#,
            r#"{ "data": { "roster": [] } }"#,
        ] {
            assert_eq!(profile_roster(other.as_bytes()), None, "{other}");
        }
    }
}
