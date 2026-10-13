//! Password rules and hashing.
//!
//! A password is any [`MIN_CHARS`] to [`MAX_CHARS`] characters, with no
//! rules about which kinds, as long as it is not the username. It is
//! NFKC-normalised before it is counted, hashed or compared, so the same
//! password typed on different keyboards matches.
//!
//! Hashes are Argon2id with OWASP's recommended minimum cost (19 MiB, two
//! passes, one lane), in the PHC string format, which records the cost: a
//! hash made with an older cost is replaced on the next sign-in. Hashing
//! runs on a blocking thread, at most [`MAX_CONCURRENT`] at a time, so a
//! burst of sign-ins cannot exhaust memory or stall the server; a slot is
//! held until its hash is done, even if the request that wanted it has
//! gone. A password longer than [`MAX_BYTES`] is refused before any of that
//! work.

use std::sync::{Arc, OnceLock};

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;
use unicode_normalization::UnicodeNormalization;

use crate::error::ApiError;
use crate::stored::blocking;

/// Shortest password allowed, in characters.
pub const MIN_CHARS: usize = 8;
/// Longest password allowed, in characters.
pub const MAX_CHARS: usize = 128;
/// Longest password read at all, in bytes, before normalising: far more than
/// [`MAX_CHARS`] characters could take, however they were typed.
pub const MAX_BYTES: usize = 4 * 1024;
/// Most hashes computed at once; each takes 19 MiB.
pub const MAX_CONCURRENT: usize = 4;

/// The hasher: Argon2id, version 0x13, at OWASP's recommended minimum cost.
fn argon2() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::DEFAULT)
}

/// `password` as it is counted, hashed and compared, unless it is too long
/// to be one.
pub fn normalize(password: &str) -> Option<String> {
    let normalized: String = (password.len() <= MAX_BYTES).then(|| password.nfkc().collect())?;
    (normalized.chars().count() <= MAX_CHARS).then_some(normalized)
}

/// A new password, normalised, or `400` if it breaks the rules.
pub fn check_new(password: &str, username: &str) -> Result<String, ApiError> {
    let length = || {
        ApiError::bad_request(format!(
            "a password must be {MIN_CHARS} to {MAX_CHARS} characters long"
        ))
    };
    let password = normalize(password).ok_or_else(length)?;
    if password.chars().count() < MIN_CHARS {
        return Err(length());
    }
    if password.eq_ignore_ascii_case(username) {
        return Err(ApiError::bad_request("a password must not be the username"));
    }
    Ok(password)
}

/// What [`Passwords::verify`] found.
pub enum Verdict {
    /// The password does not match.
    Wrong,
    /// It matches. If the hash was made with an older cost, `rehash` is a
    /// new one to store in its place.
    Right { rehash: Option<String> },
}

/// Hashes and checks passwords, a few at a time.
#[derive(Debug)]
pub struct Passwords {
    permits: Arc<Semaphore>,
}

impl Default for Passwords {
    fn default() -> Self {
        Passwords {
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }
}

impl Passwords {
    /// Runs `work` on a blocking thread once a slot is free. The slot goes
    /// with the work, so it stays taken until the work is done even if this
    /// future is dropped.
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .map_err(ApiError::internal)?;
        blocking(move || {
            let _permit = permit;
            work()
        })
        .await?
    }

    /// Hashes a normalised password.
    pub async fn hash(&self, password: String) -> Result<String, ApiError> {
        self.run(move || hash(&password)).await
    }

    /// Checks a normalised password against a stored hash.
    pub async fn verify(&self, password: String, stored: String) -> Result<Verdict, ApiError> {
        self.run(move || verify(&password, &stored)).await
    }

    /// Takes as long as [`verify`](Self::verify) does, for a sign-in to an
    /// account that does not exist, so that timing cannot tell the two
    /// apart.
    pub async fn verify_nobody(&self, password: String) -> Result<(), ApiError> {
        self.run(move || {
            static DECOY: OnceLock<String> = OnceLock::new();
            let decoy = match DECOY.get() {
                Some(decoy) => decoy,
                None => {
                    let decoy = hash("not anyone's password")?;
                    DECOY.get_or_init(|| decoy)
                }
            };
            verify(&password, decoy).map(|_| ())
        })
        .await
    }
}

fn hash(password: &str) -> Result<String, ApiError> {
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(ApiError::internal)?;
    let salt = SaltString::encode_b64(&salt).map_err(ApiError::internal)?;
    let hash = argon2()
        .hash_password(password.as_bytes(), &salt)
        .map_err(ApiError::internal)?;
    Ok(hash.to_string())
}

fn verify(password: &str, stored: &str) -> Result<Verdict, ApiError> {
    let parsed = PasswordHash::new(stored)
        .map_err(|e| ApiError::internal(format!("a stored password hash is unreadable: {e}")))?;
    if argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_err()
    {
        return Ok(Verdict::Wrong);
    }
    let rehash = if is_current(&parsed) {
        None
    } else {
        Some(hash(password)?)
    };
    Ok(Verdict::Right { rehash })
}

/// Whether a hash was made the way [`argon2`] makes them now.
fn is_current(hash: &PasswordHash<'_>) -> bool {
    hash.algorithm == Algorithm::Argon2id.ident()
        && hash.version == Some(Version::V0x13.into())
        && Params::try_from(hash).is_ok_and(|p| {
            (p.m_cost(), p.t_cost(), p.p_cost())
                == (
                    Params::DEFAULT.m_cost(),
                    Params::DEFAULT.t_cost(),
                    Params::DEFAULT.p_cost(),
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_is_owasps_minimum() {
        let hash = hash("correct horse battery staple").unwrap();
        assert!(
            hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{hash}"
        );
        assert!(is_current(&PasswordHash::new(&hash).unwrap()));
    }

    #[tokio::test]
    async fn a_dropped_request_keeps_its_slot_until_the_hash_is_done() {
        let passwords = Passwords::default();
        // Polled once, so the hash has started, then dropped.
        let started = tokio::time::timeout(
            std::time::Duration::ZERO,
            passwords.hash("correct horse".into()),
        )
        .await;
        assert!(started.is_err());
        assert_eq!(passwords.permits.available_permits(), MAX_CONCURRENT - 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while passwords.permits.available_permits() < MAX_CONCURRENT {
            assert!(
                std::time::Instant::now() < deadline,
                "the slot was never freed"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[test]
    fn huge_passwords_are_refused_before_normalising() {
        assert_eq!(normalize(&"x".repeat(MAX_CHARS)).unwrap().len(), MAX_CHARS);
        assert_eq!(normalize(&"x".repeat(MAX_CHARS + 1)), None);
        // U+FDFA is 3 bytes that normalise to 18 characters.
        assert_eq!(normalize(&"\u{FDFA}".repeat(MAX_BYTES)), None);
        assert_eq!(normalize(&"\u{FDFA}".repeat(8)), None);
    }

    #[test]
    fn rules_count_normalised_characters() {
        assert!(check_new("short", "doctor").is_err());
        assert!(check_new(&"x".repeat(MAX_CHARS + 1), "doctor").is_err());
        assert!(check_new("Doctor12", "doctor12").is_err());
        assert_eq!(check_new("eight ch", "doctor").unwrap(), "eight ch");
        // Seven characters, but "ﬁ" becomes "fi": eight once normalised.
        assert_eq!(check_new("ﬁrewall", "doctor").unwrap(), "firewall");
        // Eight characters of four bytes each are eight, not 32.
        assert!(check_new(&"🍰".repeat(8), "doctor").is_ok());
    }

    #[test]
    fn verifying_matches_and_upgrades_old_hashes() {
        let stored = hash("correct horse").unwrap();
        assert!(matches!(
            verify("correct horse", &stored).unwrap(),
            Verdict::Right { rehash: None }
        ));
        assert!(matches!(
            verify("wrong horse", &stored).unwrap(),
            Verdict::Wrong
        ));

        let weak = Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(8 * 1024, 1, 1, None).unwrap(),
        );
        let salt = SaltString::encode_b64(&[7; 16]).unwrap();
        let old = weak
            .hash_password(b"correct horse", &salt)
            .unwrap()
            .to_string();
        let Verdict::Right { rehash: Some(new) } = verify("correct horse", &old).unwrap() else {
            panic!("an old hash should be replaced");
        };
        assert!(new.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{new}");
        assert!(matches!(
            verify("correct horse", &new).unwrap(),
            Verdict::Right { rehash: None }
        ));
    }
}
