//! Accounts and their sign-in sessions.
//!
//! The store keeps what it is given: deciding what makes a valid username
//! or password, hashing passwords and minting session tokens is the API's
//! job. A session is kept as the SHA-256 digest of its token, never the
//! token itself.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Store, StoreError, rfc3339, timestamp};

/// An account, as its owner may see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
pub struct User {
    /// Never changes.
    pub id: Uuid,
    /// As the user typed it; unique regardless of case.
    pub username: String,
    /// Optional and unverified.
    pub email: Option<String>,
    /// RFC 3339, UTC, milliseconds.
    #[serde(serialize_with = "rfc3339")]
    pub created_at: DateTime<Utc>,
}

impl User {
    /// A new account with a fresh id.
    pub fn new(username: String, email: Option<String>) -> Self {
        User {
            id: Uuid::new_v4(),
            username,
            email,
            created_at: timestamp(),
        }
    }
}

/// An account and its password hash, for signing in.
#[derive(FromRow)]
struct CredentialsRow {
    #[sqlx(flatten)]
    user: User,
    password_hash: String,
}

/// A sign-in session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// SHA-256 of the session token.
    pub digest: [u8; 32],
    /// Whose session it is.
    pub user_id: Uuid,
    /// When it started.
    pub created_at: DateTime<Utc>,
    /// When it stops working.
    pub expires_at: DateTime<Utc>,
}

/// The unique index that keeps usernames unique regardless of case.
const USERNAME_KEY: &str = "users_username_key";

impl Store {
    /// Stores a new account. Returns `false`, storing nothing, if its
    /// username is taken in any case.
    pub async fn create_user(&self, user: &User, password_hash: &str) -> Result<bool, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO users (id, username, email, password_hash, created_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user.id)
        .bind(&user.username)
        .bind(&user.email)
        .bind(password_hash)
        .bind(user.created_at)
        .execute(&self.pool)
        .await;
        match inserted {
            Ok(_) => Ok(true),
            Err(sqlx::Error::Database(e))
                if e.is_unique_violation() && e.constraint() == Some(USERNAME_KEY) =>
            {
                Ok(false)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The account with this username, in any case, and its password hash.
    pub async fn credentials(&self, username: &str) -> Result<Option<(User, String)>, StoreError> {
        let row: Option<CredentialsRow> = sqlx::query_as(
            "SELECT id, username, email, created_at, password_hash \
             FROM users WHERE lower(username COLLATE \"C\") = lower($1 COLLATE \"C\")",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| (row.user, row.password_hash)))
    }

    /// Replaces an account's password hash.
    pub async fn set_password_hash(
        &self,
        user_id: Uuid,
        password_hash: &str,
    ) -> Result<(), StoreError> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(user_id)
            .bind(password_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Starts a session.
    pub async fn create_session(&self, session: &Session) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO sessions (token_digest, user_id, created_at, expires_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&session.digest[..])
        .bind(session.user_id)
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The account whose session has this digest, unless there is no such
    /// session or it had expired by `now`.
    pub async fn session_user(
        &self,
        digest: &[u8; 32],
        now: DateTime<Utc>,
    ) -> Result<Option<User>, StoreError> {
        Ok(sqlx::query_as(
            "SELECT u.id, u.username, u.email, u.created_at \
             FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_digest = $1 AND s.expires_at > $2",
        )
        .bind(&digest[..])
        .bind(now)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Ends a session. Returns whether there was one.
    pub async fn delete_session(&self, digest: &[u8; 32]) -> Result<bool, StoreError> {
        let done = sqlx::query("DELETE FROM sessions WHERE token_digest = $1")
            .bind(&digest[..])
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Removes every session that had expired by `now`. Returns how many.
    pub async fn delete_expired_sessions(&self, now: DateTime<Utc>) -> Result<u64, StoreError> {
        let done = sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use sqlx::PgPool;

    use super::*;

    fn session(user: &User, byte: u8, expires_in: TimeDelta) -> Session {
        let now = Utc::now();
        Session {
            digest: [byte; 32],
            user_id: user.id,
            created_at: now,
            expires_at: now + expires_in,
        }
    }

    #[sqlx::test]
    async fn usernames_are_unique_regardless_of_case(pool: PgPool) {
        let store = Store::from_pool(pool);
        let doctor = User::new("Doctor".into(), Some("doc@example.com".into()));
        assert!(store.create_user(&doctor, "hash-1").await.unwrap());
        let twin = User::new("dOCTOR".into(), None);
        assert!(!store.create_user(&twin, "hash-2").await.unwrap());

        let (user, hash) = store.credentials("DOCTOR").await.unwrap().unwrap();
        assert_eq!((user, hash.as_str()), (doctor.clone(), "hash-1"));
        assert_eq!(store.credentials("nobody").await.unwrap(), None);

        store.set_password_hash(doctor.id, "hash-3").await.unwrap();
        let (_, hash) = store.credentials("doctor").await.unwrap().unwrap();
        assert_eq!(hash, "hash-3");
    }

    #[sqlx::test]
    async fn sessions_live_until_they_expire_or_end(pool: PgPool) {
        let store = Store::from_pool(pool);
        let user = User::new("amiya".into(), None);
        store.create_user(&user, "hash").await.unwrap();
        let live = session(&user, 1, TimeDelta::days(1));
        let stale = session(&user, 2, TimeDelta::seconds(-1));
        store.create_session(&live).await.unwrap();
        store.create_session(&stale).await.unwrap();

        let now = Utc::now();
        assert_eq!(
            store.session_user(&live.digest, now).await.unwrap(),
            Some(user.clone())
        );
        assert_eq!(store.session_user(&stale.digest, now).await.unwrap(), None);
        assert_eq!(store.session_user(&[3; 32], now).await.unwrap(), None);

        assert_eq!(store.delete_expired_sessions(now).await.unwrap(), 1);
        assert!(store.delete_session(&live.digest).await.unwrap());
        assert!(!store.delete_session(&live.digest).await.unwrap());
        assert_eq!(store.session_user(&live.digest, now).await.unwrap(), None);
    }
}
