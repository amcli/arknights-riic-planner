//! Accounts and signing in.
//!
//! ```text
//! POST /api/v1/auth/register   { username, password, email? } → 201, the account, signed in
//! POST /api/v1/auth/login      { username, password }         → 200, the account, signed in
//! POST /api/v1/auth/logout                                    → 204, signed out
//! GET  /api/v1/auth/me                                        → the account signed in, or 401
//! ```
//!
//! A username is 3 to 32 letters, digits, `_` or `-`, unique regardless of
//! case. The email is optional and only checked for shape. Passwords are
//! described in [`password`], sessions in [`session`]. A wrong username and
//! a wrong password get the same `401`, after the same amount of work.
//! Handlers that need the account take a [`CurrentUser`].

pub mod password;
pub mod session;

use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use ak_store::accounts::{Session, User};

use crate::AppState;
use crate::error::{ApiError, Body};
use password::Verdict;
use session::Token;

/// Shortest username.
pub const MIN_USERNAME: usize = 3;
/// Longest username.
pub const MAX_USERNAME: usize = 32;
/// Longest email address (RFC 5321's limit on a path).
pub const MAX_EMAIL: usize = 254;

/// `POST /auth/register` body. Not `Debug`: it holds a password.
#[derive(Deserialize)]
pub struct Registration {
    username: String,
    password: String,
    #[serde(default)]
    email: Option<String>,
}

/// `POST /auth/login` body. Not `Debug`: it holds a password.
#[derive(Deserialize)]
pub struct SignIn {
    username: String,
    password: String,
}

/// The account a request's session belongs to. As an extractor, it
/// answers `401` when the request is not signed in.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, s: &AppState) -> Result<Self, ApiError> {
        let not_signed_in = || ApiError::unauthorized("not signed in");
        let token = Token::from_headers(&parts.headers).ok_or_else(not_signed_in)?;
        let user = s.store.session_user(&token.digest(), Utc::now()).await?;
        user.map(CurrentUser).ok_or_else(not_signed_in)
    }
}

/// `raw`, if it can be a username.
fn check_username(raw: &str) -> Result<&str, ApiError> {
    let valid = (MIN_USERNAME..=MAX_USERNAME).contains(&raw.len())
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if valid {
        Ok(raw)
    } else {
        Err(ApiError::bad_request(format!(
            "a username is {MIN_USERNAME} to {MAX_USERNAME} letters, digits, `_` or `-`"
        )))
    }
}

/// `raw`, trimmed, if it has the shape of an email address; nothing if it
/// is empty. Whether it reaches anyone is not checked.
fn check_email(raw: Option<String>) -> Result<Option<String>, ApiError> {
    let Some(email) = raw.map(|e| e.trim().to_owned()).filter(|e| !e.is_empty()) else {
        return Ok(None);
    };
    let shaped = email.len() <= MAX_EMAIL
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if shaped {
        Ok(Some(email))
    } else {
        Err(ApiError::bad_request("that is not an email address"))
    }
}

/// Starts a session for `user_id` and returns the `Set-Cookie` value that
/// hands it over. A session the request already carried ends: signing in
/// always gets a fresh token.
async fn sign_in(
    s: &AppState,
    request: &HeaderMap,
    user_id: Uuid,
) -> Result<HeaderValue, ApiError> {
    if let Some(old) = Token::from_headers(request) {
        s.store.delete_session(&old.digest()).await?;
    }
    let token = Token::generate()?;
    let now = Utc::now();
    s.store
        .create_session(&Session {
            digest: token.digest(),
            user_id,
            created_at: now,
            expires_at: now + session::LIFETIME,
        })
        .await?;
    Ok(token.set_cookie())
}

/// `POST /api/v1/auth/register`: creates an account and signs it in.
pub async fn register(
    State(s): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<Registration>,
) -> Result<Response, ApiError> {
    let username = check_username(&body.username)?.to_owned();
    let email = check_email(body.email)?;
    let password = password::check_new(&body.password, &username)?;
    let hash = s.passwords.hash(password).await?;
    let user = User::new(username, email);
    if !s.store.create_user(&user, &hash).await? {
        return Err(ApiError::conflict(format!(
            "the username {:?} is taken",
            user.username
        )));
    }
    let cookie = sign_in(&s, &headers, user.id).await?;
    Ok((
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(user),
    )
        .into_response())
}

/// `POST /api/v1/auth/login`: signs in.
pub async fn login(
    State(s): State<AppState>,
    headers: HeaderMap,
    Body(body): Body<SignIn>,
) -> Result<Response, ApiError> {
    let wrong = || ApiError::unauthorized("wrong username or password");
    let Some(password) = password::normalize(&body.password) else {
        return Err(wrong());
    };
    let found = match check_username(&body.username) {
        Ok(username) => s.store.credentials(username).await?,
        Err(_) => None,
    };
    let Some((user, stored)) = found else {
        s.passwords.verify_nobody(password).await?;
        return Err(wrong());
    };
    let Verdict::Right { rehash } = s.passwords.verify(password, stored).await? else {
        return Err(wrong());
    };
    if let Some(hash) = rehash {
        // An upgrade that fails is tried again at the next sign-in.
        if let Err(e) = s.store.set_password_hash(user.id, &hash).await {
            tracing::warn!(user = %user.id, "cannot store an upgraded password hash: {e}");
        }
    }
    let cookie = sign_in(&s, &headers, user.id).await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(user)).into_response())
}

/// `POST /api/v1/auth/logout`: ends the request's session, if it has one.
pub async fn logout(State(s): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = Token::from_headers(&headers) {
        s.store.delete_session(&token.digest()).await?;
    }
    let cleared = [(header::SET_COOKIE, session::clear_cookie())];
    Ok((StatusCode::NO_CONTENT, cleared).into_response())
}

/// `GET /api/v1/auth/me`: the account signed in.
pub async fn me(CurrentUser(user): CurrentUser) -> Json<User> {
    Json(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_short_and_plain() {
        for good in [
            "ash",
            "Doctor_42",
            "rhodes-island",
            &"x".repeat(MAX_USERNAME),
        ] {
            assert!(check_username(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "ab",
            "has space",
            "dot.ted",
            "émile",
            &"x".repeat(MAX_USERNAME + 1),
        ] {
            assert!(check_username(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn emails_are_optional_and_only_shape_checked() {
        assert_eq!(check_email(None).unwrap(), None);
        assert_eq!(check_email(Some("  ".into())).unwrap(), None);
        assert_eq!(
            check_email(Some(" doc@rhodes.example ".into())).unwrap(),
            Some("doc@rhodes.example".into())
        );
        for bad in [
            "doc",
            "@rhodes.example",
            "doc@",
            "doc@localhost",
            "doc@.example",
            "a@b@c.d",
            "d oc@x.y",
        ] {
            assert!(check_email(Some(bad.into())).is_err(), "{bad}");
        }
    }
}
