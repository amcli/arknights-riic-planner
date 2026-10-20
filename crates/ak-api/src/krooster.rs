//! Rosters fetched from Krooster by username.
//!
//! Krooster's public profile route, `/api/u/{username}`, answers with a
//! user's account details, support units and roster (see
//! [`ak_data::import::krooster`]). It sends no CORS headers, so a page on
//! another origin cannot read it; `GET /api/v1/import/krooster/{username}`
//! fetches it here instead and answers with the roster alone,
//! `{ data: { roster } }`, ready to store with source `krooster`. The
//! account details and supports are dropped as soon as they arrive.
//!
//! Answers are kept in memory for a while, so that fetching the same
//! roster again, by anyone, does not ask Krooster again: a roster for
//! [`FRESH_FOR`], and "no such user" for [`MISSING_FOR`], so that a typo
//! costs Krooster one request and a new account is found soon. Krooster's
//! own failures and answers that are not a profile are never kept. Requests
//! for a name being fetched wait for that fetch rather than starting their
//! own, and at most [`MAX_FETCHES`] fetches run at once. A fetch carries
//! on, holding its place, until Krooster has answered, even if every request
//! waiting on it has gone, and its answer is kept for the next. A roster
//! served from memory says how old it is in `Age`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use moka::Expiry;
use moka::future::Cache;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use ureq::tls::{TlsConfig, TlsProvider};

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::ApiError;
use crate::stored::{blocking, to_completion};

/// Where Krooster lives; the bare `krooster.com` redirects here.
pub const KROOSTER_URL: &str = "https://www.krooster.com";

/// How long a roster is served from memory before Krooster is asked again.
pub const FRESH_FOR: Duration = Duration::from_secs(5 * 60);
/// How long "no such user" is remembered.
pub const MISSING_FOR: Duration = Duration::from_secs(60);
/// Fetches from Krooster at once; more wait their turn.
pub const MAX_FETCHES: usize = 4;
/// Most memory the remembered answers may take, in bytes: about 300 full
/// rosters.
const MAX_KEPT_BYTES: u64 = 32 * 1024 * 1024;

/// Longest a fetch may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Largest answer read. A full roster is about 100 KB.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Longest Krooster username: its settings page allows 31 characters of
/// letters, digits, `_` and `-`, and its generated names stay within 32.
const MAX_USERNAME: usize = 32;

/// What Krooster answered for a username.
#[derive(Debug, Clone)]
enum Answer {
    /// The roster, as `{ "data": { "roster": … } }` JSON, and when it came.
    Found { json: Bytes, at: Instant },
    /// No such user.
    Missing,
}

/// How long each kind of answer is kept.
struct KeepFor {
    found: Duration,
    missing: Duration,
}

impl Expiry<String, Answer> for KeepFor {
    fn expire_after_create(&self, _: &String, answer: &Answer, _: Instant) -> Option<Duration> {
        Some(match answer {
            Answer::Found { .. } => self.found,
            Answer::Missing => self.missing,
        })
    }
}

/// Room for answers, by name, each kept as long as `keep` says and all of
/// them within [`MAX_KEPT_BYTES`].
fn kept(keep: KeepFor) -> Cache<String, Answer> {
    Cache::builder()
        .max_capacity(MAX_KEPT_BYTES)
        .weigher(|name: &String, answer: &Answer| -> u32 {
            let size = match answer {
                Answer::Found { json, .. } => json.len(),
                Answer::Missing => 0,
            };
            u32::try_from(name.len() + size).unwrap_or(u32::MAX)
        })
        .expire_after(keep)
        .build()
}

/// Reads Krooster's public profiles.
#[derive(Debug)]
pub struct Krooster {
    base: String,
    agent: ureq::Agent,
    /// Answers by lower-case username.
    kept: Cache<String, Answer>,
    /// Places for fetches in flight.
    fetches: Arc<Semaphore>,
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
            kept: kept(KeepFor {
                found: FRESH_FOR,
                missing: MISSING_FOR,
            }),
            fetches: Arc::new(Semaphore::new(MAX_FETCHES)),
        }
    }

    /// The roster in `username`'s public profile, as
    /// `{ "data": { "roster": … } }` JSON, and how long ago Krooster sent
    /// it: from memory if it is recent enough, otherwise from Krooster.
    pub async fn roster(&self, username: &str) -> Result<(Bytes, Duration), ApiError> {
        let name = check_username(username)?;
        // Krooster looks usernames up in lower case.
        let key = name.to_ascii_lowercase();
        // Looked up on a task of its own, so that a fetch is not given up,
        // nor its answer lost, when the request that started it is dropped.
        let (kept, fetch) = (self.kept.clone(), self.fetch(key.clone()));
        let answer = to_completion(async move { kept.try_get_with(key, fetch).await })
            .await?
            .map_err(|e| ApiError::clone(&e))?;
        match answer {
            Answer::Found { json, at } => Ok((json, at.elapsed())),
            Answer::Missing => Err(ApiError::not_found(format!(
                "no Krooster user named {name:?}"
            ))),
        }
    }

    /// Asks Krooster about `name`, once a place is free. Blocks a thread
    /// until Krooster answers or the timeout passes.
    fn fetch(&self, name: String) -> impl Future<Output = Result<Answer, ApiError>> + use<> {
        let (places, agent) = (self.fetches.clone(), self.agent.clone());
        let url = format!("{}/api/u/{name}", self.base);
        async move {
            let place = places.acquire_owned().await.map_err(ApiError::internal)?;
            blocking(move || {
                let _place = place;
                ask(&agent, &url, &name)
            })
            .await?
        }
    }
}

/// What Krooster at `url` says about `name`.
fn ask(agent: &ureq::Agent, url: &str, name: &str) -> Result<Answer, ApiError> {
    let mut response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(Answer::Missing),
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
    let roster = profile_roster(&body).ok_or_else(|| {
        ApiError::bad_gateway(format!(
            "Krooster's answer for {name:?} is not a Krooster profile"
        ))
    })?;
    Ok(Answer::Found {
        json: Bytes::from(roster.to_string()),
        at: Instant::now(),
    })
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
/// user's public profile, with its age in seconds as `Age`.
pub async fn roster(
    State(s): State<AppState>,
    _: CurrentUser,
    Path(username): Path<String>,
) -> Result<Response, ApiError> {
    let (json, age) = s.krooster.roster(&username).await?;
    let headers = [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ),
        (header::AGE, HeaderValue::from(age.as_secs())),
    ];
    Ok((headers, json).into_response())
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

    #[tokio::test]
    async fn each_kind_of_answer_is_kept_as_long_as_it_should_be() {
        let kept = kept(KeepFor {
            found: Duration::from_secs(1),
            missing: Duration::from_millis(50),
        });
        let found = Answer::Found {
            json: Bytes::from_static(b"{}"),
            at: Instant::now(),
        };
        kept.insert("found".into(), found).await;
        kept.insert("missing".into(), Answer::Missing).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(kept.get("found").await.is_some());
        assert!(kept.get("missing").await.is_none());
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(kept.get("found").await.is_none());
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
