//! Session tokens, the cookie that carries them, and the guard against
//! cross-site requests.
//!
//! A token is 32 random bytes, sent to the browser hex-encoded in the
//! [`COOKIE`] cookie: `HttpOnly` (page scripts cannot read it), `Secure`,
//! `SameSite=Lax` (other sites' fetches, frames and form posts do not carry
//! it), and with the `__Host-` prefix, which makes browsers refuse it
//! unless it is all of that and scoped to this host's `/`. The store keeps
//! only its SHA-256 digest. A session lasts [`LIFETIME`] from sign-in.
//!
//! The cookie is found by its exact name. Cookie names are not
//! percent-decoded: a cookie named `%5F%5FHost-session` is not the session
//! cookie, and since the browser did not hold it to the `__Host-` rules,
//! another site or a sibling subdomain could have planted it.

use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{TimeDelta, Utc};
use cookie::{Cookie, SameSite};
use sha2::{Digest, Sha256};

use ak_store::Store;

use crate::error::ApiError;

/// The session cookie's name.
pub const COOKIE: &str = "__Host-session";
/// How long a session lasts from sign-in.
pub const LIFETIME: TimeDelta = TimeDelta::days(30);
/// How often expired sessions are removed from the store.
pub const SWEEP_EVERY: Duration = Duration::from_secs(60 * 60);

/// A session token. Deliberately not `Debug`: it is a credential.
pub struct Token([u8; 32]);

impl Token {
    /// A fresh random token.
    pub fn generate() -> Result<Self, ApiError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(ApiError::internal)?;
        Ok(Token(bytes))
    }

    /// The token a request's session cookie carries, if it carries one that
    /// could be a token.
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let mut bytes = [0; 32];
        hex::decode_to_slice(session_cookie(headers)?, &mut bytes).ok()?;
        Some(Token(bytes))
    }

    /// What the store keeps in its place.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.0).into()
    }

    /// The `Set-Cookie` value that hands it to the browser.
    pub fn set_cookie(&self) -> HeaderValue {
        let mut cookie = build(hex::encode(self.0));
        cookie.set_max_age(cookie::time::Duration::seconds(LIFETIME.num_seconds()));
        header_value(&cookie)
    }
}

/// The `Set-Cookie` value that makes the browser forget its session cookie.
pub fn clear_cookie() -> HeaderValue {
    let mut cookie = build(String::new());
    cookie.make_removal();
    header_value(&cookie)
}

fn build(value: String) -> Cookie<'static> {
    Cookie::build((COOKIE, value))
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .path("/")
        .build()
}

fn header_value(cookie: &Cookie<'_>) -> HeaderValue {
    HeaderValue::try_from(cookie.to_string())
        .expect("hex, a fixed name and fixed attributes make a valid header")
}

/// The value of the request's session cookie, matched by its exact,
/// undecoded name.
fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|line| line.to_str().ok())
        .flat_map(|line| line.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE).then_some(value)
        })
}

/// Middleware: refuses, with `403`, a request from another site that would
/// act on a session (any method but `GET`, `HEAD`, `OPTIONS` or `TRACE`,
/// carrying the session cookie) or start or end one (any such method under
/// `/api/v1/auth/`). `SameSite=Lax` already keeps other sites' fetches and
/// form posts from carrying the cookie; this also covers sites that count
/// as the same site, such as another port on the same host. Browsers say
/// where a request came from in `Sec-Fetch-Site`; requests without it
/// (other clients, and browsers too old to send it) pass.
pub async fn guard(req: Request, next: Next) -> Response {
    let acts_for_a_session = !req.method().is_safe()
        && (req.uri().path().starts_with("/api/v1/auth/")
            || session_cookie(req.headers()).is_some());
    if acts_for_a_session && is_cross_site(req.headers()) {
        return ApiError::forbidden("refused a request from another site").into_response();
    }
    next.run(req).await
}

fn is_cross_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .is_some_and(|site| site != "same-origin" && site != "none")
}

/// Removes expired sessions now and every [`SWEEP_EVERY`] after, until the
/// task is dropped.
pub async fn sweep(store: Store) {
    let mut every = tokio::time::interval(SWEEP_EVERY);
    loop {
        every.tick().await;
        match store.delete_expired_sessions(Utc::now()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(sessions = n, "removed expired sessions"),
            Err(e) => tracing::warn!("cannot remove expired sessions: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sending(cookies: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_str(cookies).unwrap());
        headers
    }

    /// `name=value` from a `Set-Cookie` value.
    fn pair(set: &HeaderValue) -> String {
        set.to_str().unwrap().split(';').next().unwrap().to_owned()
    }

    #[test]
    fn tokens_round_trip_through_their_cookie() {
        let token = Token::generate().unwrap();
        let set = token.set_cookie();
        let line = set.to_str().unwrap();
        for attribute in [
            "HttpOnly",
            "Secure",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=2592000",
        ] {
            assert!(line.contains(attribute), "{line}");
        }
        assert!(!line.contains("Domain"), "{line}");

        let back = Token::from_headers(&sending(&format!("theme=dark; {}", pair(&set)))).unwrap();
        assert_eq!(back.digest(), token.digest());
        assert_ne!(token.digest(), Token::generate().unwrap().digest());

        for bad in ["", "zz", &"0".repeat(63), &"0".repeat(66)] {
            assert!(Token::from_headers(&sending(&format!("{COOKIE}={bad}"))).is_none());
        }
        assert!(Token::from_headers(&HeaderMap::new()).is_none());
    }

    #[test]
    fn only_the_exact_name_is_the_session_cookie() {
        let real = Token::generate().unwrap();
        let planted = Token::generate().unwrap();
        let real_pair = pair(&real.set_cookie());
        let planted_value = hex::encode(planted.0);
        for cookies in [
            format!("{real_pair}; %5F%5FHost-session={planted_value}"),
            format!("%5F%5FHost-session={planted_value}; {real_pair}"),
        ] {
            let found = Token::from_headers(&sending(&cookies)).unwrap();
            assert_eq!(found.digest(), real.digest(), "{cookies}");
        }
        let alone = format!("%5F%5FHost-session={planted_value}");
        assert!(Token::from_headers(&sending(&alone)).is_none());
    }

    #[test]
    fn clearing_keeps_the_attributes_the_prefix_requires() {
        let set = clear_cookie();
        let line = set.to_str().unwrap();
        for attribute in ["Secure", "Path=/", "Max-Age=0"] {
            assert!(line.contains(attribute), "{line}");
        }
    }
}
