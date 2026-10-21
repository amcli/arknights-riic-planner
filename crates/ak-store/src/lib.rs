//! Persistence (Layer 6): a document store for rosters, bases and solves,
//! and the accounts that sign in ([`accounts`]), kept in Postgres.
//!
//! Every stored thing is a [`Document`]: an id, the account that owns it,
//! timestamps, an optional name, a `schema_version`, and a JSON body. Reads
//! and writes name the owner, so one account never sees another's
//! documents. Bodies are never decomposed
//! into columns; the domain still moves too fast for that. Old bodies are
//! migrated on read ([`migrate`]), never in place.
//!
//! [`Store`] is a pool of connections to one database. Its tables come
//! from the SQL migrations in `migrations/`, embedded as [`MIGRATOR`] and
//! applied by [`apply_migrations`] (`ak-api migrate`) rather than by the
//! server, so the server can connect as a role that reads and writes rows
//! but cannot change the schema. [`Store::check_schema`] refuses a database
//! whose migrations do not match this build's.

#![forbid(unsafe_code)]

pub mod accounts;

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, SubsecRound, Utc};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sqlx::migrate::{Migrate, MigrateError, Migrator};
use sqlx::postgres::{PgConnection, PgPool, PgPoolOptions};
use sqlx::{Connection, FromRow};
use uuid::Uuid;

/// The document schema this build writes and the newest it can read.
pub const SCHEMA_VERSION: u32 = 1;

/// The schema migrations, embedded from `migrations/`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// Which kind of document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Collection {
    /// Owned operators and their promotion state (`ak_domain::Roster`).
    Rosters,
    /// Room layouts (`ak_domain::BaseConfig`).
    Bases,
    /// Solve jobs: request, status, and result.
    Solves,
}

impl Collection {
    /// The `collection` column's value and URL segment.
    pub const fn as_str(self) -> &'static str {
        match self {
            Collection::Rosters => "rosters",
            Collection::Bases => "bases",
            Collection::Solves => "solves",
        }
    }
}

impl fmt::Display for Collection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stored thing.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Unique within its collection.
    pub id: Uuid,
    /// The account it belongs to.
    pub owner: Uuid,
    /// The schema the body was written with.
    pub schema_version: u32,
    /// When it was stored, to the millisecond.
    pub created_at: DateTime<Utc>,
    /// When it last changed, to the millisecond.
    pub updated_at: DateTime<Utc>,
    /// Display name, if any.
    pub name: Option<String>,
    /// The payload.
    pub body: Value,
}

impl Document {
    /// A new document for `owner`, with a fresh id and the current schema
    /// version.
    pub fn new(owner: Uuid, name: Option<String>, body: Value) -> Self {
        let now = timestamp();
        Document {
            id: Uuid::new_v4(),
            owner,
            schema_version: SCHEMA_VERSION,
            created_at: now,
            updated_at: now,
            name,
            body,
        }
    }

    /// Bumps `updated_at`.
    pub fn touch(&mut self) {
        self.updated_at = timestamp();
    }

    /// The listing view.
    pub fn meta(&self) -> DocumentMeta {
        DocumentMeta {
            id: self.id,
            name: self.name.clone(),
            schema_version: self.schema_version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// What a listing shows about a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromRow)]
pub struct DocumentMeta {
    /// Unique within its collection.
    pub id: Uuid,
    /// Display name, if any.
    pub name: Option<String>,
    /// The schema the body was written with.
    #[sqlx(try_from = "i32")]
    pub schema_version: u32,
    /// RFC 3339, UTC, milliseconds.
    #[serde(serialize_with = "rfc3339")]
    pub created_at: DateTime<Utc>,
    /// RFC 3339, UTC, milliseconds.
    #[serde(serialize_with = "rfc3339")]
    pub updated_at: DateTime<Utc>,
}

/// A whole row of the `documents` table.
#[derive(FromRow)]
struct Row {
    #[sqlx(flatten)]
    meta: DocumentMeta,
    owner_id: Uuid,
    body: Value,
}

impl From<Row> for Document {
    fn from(row: Row) -> Self {
        let DocumentMeta {
            id,
            name,
            schema_version,
            created_at,
            updated_at,
        } = row.meta;
        Document {
            id,
            owner: row.owner_id,
            schema_version,
            created_at,
            updated_at,
            name,
            body: row.body,
        }
    }
}

/// The current time, cut to the millisecond that documents keep.
fn timestamp() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(3)
}

/// `t` as the API writes times: RFC 3339, UTC, milliseconds.
fn format_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The current time as the API writes it: RFC 3339, UTC, milliseconds.
pub fn now() -> String {
    format_time(Utc::now())
}

fn rfc3339<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format_time(*t))
}

/// Why a store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// The database could not be reached in time (it is down, or every
    /// pooled connection stayed busy), or it dropped the connection while
    /// shutting down. Worth retrying.
    Unavailable(sqlx::Error),
    /// A name or body holds text Postgres cannot store: a NUL character.
    Unstorable(sqlx::Error),
    /// Any other database failure.
    Database(sqlx::Error),
    /// The database's schema is not the one this build's migrations make.
    Schema(String),
    /// Applying the migrations failed.
    Migrate(MigrateError),
    /// The document was written by a newer build.
    NewerSchema {
        id: Uuid,
        version: u32,
        supported: u32,
    },
    /// No migration path from the document's version.
    Migration { id: Uuid, from: u32, reason: String },
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        let code = match &e {
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
                return StoreError::Unavailable(e);
            }
            sqlx::Error::Database(db) => db.code(),
            _ => None,
        };
        match code.as_deref() {
            Some(c) if c.starts_with(CONNECTION_EXCEPTION) || SHUTTING_DOWN.contains(&c) => {
                StoreError::Unavailable(e)
            }
            Some(c) if NUL_IN_TEXT.contains(&c) => StoreError::Unstorable(e),
            _ => StoreError::Database(e),
        }
    }
}

/// Postgres's code for a missing table.
const UNDEFINED_TABLE: &str = "42P01";
/// The class of Postgres codes for a broken connection.
const CONNECTION_EXCEPTION: &str = "08";
/// Postgres's codes for a server that is shutting down, crashed, or not yet
/// accepting connections.
const SHUTTING_DOWN: [&str; 3] = ["57P01", "57P02", "57P03"];
/// Postgres's codes for a NUL in `text` (invalid byte sequence) and in
/// `jsonb` (`\u0000` cannot be converted to text).
const NUL_IN_TEXT: [&str; 2] = ["22021", "22P05"];

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Unavailable(e) => write!(f, "the database is unavailable: {e}"),
            StoreError::Unstorable(e) => write!(f, "text the database cannot store: {e}"),
            StoreError::Database(e) => write!(f, "database error: {e}"),
            StoreError::Schema(reason) => write!(f, "database schema: {reason}"),
            StoreError::Migrate(e) => write!(f, "migrating the database: {e}"),
            StoreError::NewerSchema {
                id,
                version,
                supported,
            } => write!(
                f,
                "document {id} has schema version {version}, newer than the supported {supported}"
            ),
            StoreError::Migration { id, from, reason } => {
                write!(
                    f,
                    "document {id}: cannot migrate from version {from}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// Brings a document up to [`SCHEMA_VERSION`]. Documents from newer builds
/// are refused rather than guessed at.
pub fn migrate(mut doc: Document) -> Result<Document, StoreError> {
    while doc.schema_version < SCHEMA_VERSION {
        doc = step(doc)?;
    }
    if doc.schema_version > SCHEMA_VERSION {
        return Err(StoreError::NewerSchema {
            id: doc.id,
            version: doc.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(doc)
}

/// One migration step. Add an arm here when `SCHEMA_VERSION` is bumped.
fn step(doc: Document) -> Result<Document, StoreError> {
    #[allow(clippy::match_single_binding)]
    match doc.schema_version {
        from => Err(StoreError::Migration {
            id: doc.id,
            from,
            reason: "no migration is defined for this version".into(),
        }),
    }
}

/// Applies any of [`MIGRATOR`]'s migrations the database at `url` lacks.
/// The URL's role must be allowed to change the schema.
pub async fn apply_migrations(url: &str) -> Result<(), StoreError> {
    let mut conn = PgConnection::connect(url).await?;
    MIGRATOR.run(&mut conn).await.map_err(StoreError::Migrate)?;
    conn.close().await?;
    Ok(())
}

/// How [`Store`] pools its connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    /// Most connections open at once. Requests beyond that wait for one to
    /// come free.
    pub max_connections: u32,
    /// Longest a request waits for a free connection before failing with
    /// [`StoreError::Unavailable`].
    pub acquire_timeout: Duration,
}

impl PoolConfig {
    /// Default [`max_connections`](Self::max_connections): plenty for one
    /// server, and well inside Postgres's default limit of 100.
    pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;
    /// Default [`acquire_timeout`](Self::acquire_timeout).
    pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(3);
    /// Idle connections are closed after this long.
    const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
    /// Connections are replaced after this long, so none outlives a
    /// database restart or failover for long.
    const MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
            acquire_timeout: Self::DEFAULT_ACQUIRE_TIMEOUT,
        }
    }
}

/// The document store: a pool of connections to a migrated database.
/// Cloning it shares the pool.
#[derive(Debug, Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connects to the database at `url`, failing at once if it cannot be
    /// reached.
    pub async fn connect(url: &str, config: &PoolConfig) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(PoolConfig::IDLE_TIMEOUT)
            .max_lifetime(PoolConfig::MAX_LIFETIME)
            .connect(url)
            .await?;
        Ok(Store { pool })
    }

    /// A store over an existing pool.
    pub fn from_pool(pool: PgPool) -> Self {
        Store { pool }
    }

    /// Checks that every one of [`MIGRATOR`]'s migrations has been applied,
    /// unchanged, and none this build does not know.
    pub async fn check_schema(&self) -> Result<(), StoreError> {
        const HINT: &str = "run `ak-api migrate`";
        let mut conn = self.pool.acquire().await?;
        let applied = match conn.list_applied_migrations().await {
            Ok(applied) => applied,
            Err(MigrateError::Execute(sqlx::Error::Database(e)))
                if e.code().as_deref() == Some(UNDEFINED_TABLE) =>
            {
                return Err(StoreError::Schema(format!(
                    "the database has not been migrated; {HINT}"
                )));
            }
            Err(e) => return Err(history_error(e)),
        };
        if let Some(version) = conn.dirty_version().await.map_err(history_error)? {
            return Err(StoreError::Schema(format!(
                "migration {version} failed partway; repair it by hand"
            )));
        }
        for migration in MIGRATOR
            .iter()
            .filter(|m| !m.migration_type.is_down_migration())
        {
            match applied.iter().find(|a| a.version == migration.version) {
                None => {
                    return Err(StoreError::Schema(format!(
                        "migration {} ({}) has not been applied; {HINT}",
                        migration.version, migration.description
                    )));
                }
                Some(a) if a.checksum != migration.checksum => {
                    return Err(StoreError::Schema(format!(
                        "migration {} was changed after it was applied",
                        migration.version
                    )));
                }
                Some(_) => {}
            }
        }
        if let Some(a) = applied.iter().find(|a| !MIGRATOR.version_exists(a.version)) {
            return Err(StoreError::Schema(format!(
                "the database has migration {}, which this build does not know; \
                 it was migrated by a newer build",
                a.version
            )));
        }
        Ok(())
    }

    /// Closes every connection, waiting for those in use to be returned.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Stores a new document for its owner.
    pub async fn insert(&self, collection: Collection, doc: &Document) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO documents \
             (collection, id, owner_id, schema_version, created_at, updated_at, name, body) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(collection.as_str())
        .bind(doc.id)
        .bind(doc.owner)
        .bind(version_column(doc.schema_version)?)
        .bind(doc.created_at)
        .bind(doc.updated_at)
        .bind(&doc.name)
        .bind(&doc.body)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Writes a stored document's name, body, schema version and
    /// `updated_at`, if its owner still has it. Returns whether it was there;
    /// a deleted document stays deleted.
    pub async fn update(&self, collection: Collection, doc: &Document) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "UPDATE documents \
             SET schema_version = $4, updated_at = $5, name = $6, body = $7 \
             WHERE collection = $1 AND id = $2 AND owner_id = $3",
        )
        .bind(collection.as_str())
        .bind(doc.id)
        .bind(doc.owner)
        .bind(version_column(doc.schema_version)?)
        .bind(doc.updated_at)
        .bind(&doc.name)
        .bind(&doc.body)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Reads one of `owner`'s documents, migrated to the current schema.
    /// Someone else's is not there.
    pub async fn get(
        &self,
        owner: Uuid,
        collection: Collection,
        id: Uuid,
    ) -> Result<Option<Document>, StoreError> {
        let row: Option<Row> = sqlx::query_as(
            "SELECT id, owner_id, name, schema_version, created_at, updated_at, body \
             FROM documents WHERE collection = $1 AND id = $2 AND owner_id = $3",
        )
        .bind(collection.as_str())
        .bind(id)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| migrate(row.into())).transpose()
    }

    /// Lists `owner`'s documents in a collection, newest first.
    pub async fn list(
        &self,
        owner: Uuid,
        collection: Collection,
    ) -> Result<Vec<DocumentMeta>, StoreError> {
        Ok(sqlx::query_as(
            "SELECT id, name, schema_version, created_at, updated_at \
             FROM documents WHERE owner_id = $1 AND collection = $2 \
             ORDER BY created_at DESC, id",
        )
        .bind(owner)
        .bind(collection.as_str())
        .fetch_all(&self.pool)
        .await?)
    }

    /// Every one of `owner`'s documents in a collection, migrated, newest
    /// first.
    pub async fn documents(
        &self,
        owner: Uuid,
        collection: Collection,
    ) -> Result<Vec<Document>, StoreError> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, owner_id, name, schema_version, created_at, updated_at, body \
             FROM documents WHERE owner_id = $1 AND collection = $2 \
             ORDER BY created_at DESC, id",
        )
        .bind(owner)
        .bind(collection.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|row| migrate(row.into())).collect()
    }

    /// Every document in a collection, whoever owns it, migrated, oldest
    /// first. For the server's own housekeeping, such as picking up
    /// unfinished solves; never to answer a request.
    pub async fn all_documents(&self, collection: Collection) -> Result<Vec<Document>, StoreError> {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, owner_id, name, schema_version, created_at, updated_at, body \
             FROM documents WHERE collection = $1 ORDER BY created_at, id",
        )
        .bind(collection.as_str())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(|row| migrate(row.into())).collect()
    }

    /// Removes one of `owner`'s documents. Returns whether it was there.
    pub async fn delete(
        &self,
        owner: Uuid,
        collection: Collection,
        id: Uuid,
    ) -> Result<bool, StoreError> {
        let done = sqlx::query(
            "DELETE FROM documents WHERE collection = $1 AND id = $2 AND owner_id = $3",
        )
        .bind(collection.as_str())
        .bind(id)
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }
}

/// A failure to read the migration history: a database error like any other,
/// not a failed migration.
fn history_error(e: MigrateError) -> StoreError {
    match e {
        MigrateError::Execute(e) => StoreError::from(e),
        other => StoreError::Schema(other.to_string()),
    }
}

/// A schema version as the `integer` column holds it.
fn version_column(version: u32) -> Result<i32, StoreError> {
    i32::try_from(version).map_err(|e| StoreError::Database(sqlx::Error::Encode(Box::new(e))))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::accounts::User;
    use super::*;

    /// A new account's id, to own documents.
    async fn account(store: &Store, username: &str) -> Uuid {
        let user = User::new(username.into(), None);
        assert!(store.create_user(&user, "hash").await.unwrap());
        user.id
    }

    #[sqlx::test]
    async fn insert_get_list_delete_round_trip(pool: PgPool) {
        let store = Store::from_pool(pool);
        let owner = account(&store, "doctor").await;
        let a = Document::new(owner, Some("first".into()), json!({ "x": 1 }));
        let b = Document::new(owner, None, json!([1, 2, 3]));
        store.insert(Collection::Bases, &a).await.unwrap();
        store.insert(Collection::Bases, &b).await.unwrap();

        let back = store
            .get(owner, Collection::Bases, a.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(back, a);
        let elsewhere = store.get(owner, Collection::Rosters, a.id).await.unwrap();
        assert_eq!(elsewhere, None);

        let listed = store.list(owner, Collection::Bases).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .any(|m| m.id == a.id && m.name.as_deref() == Some("first"))
        );
        assert!(
            listed
                .windows(2)
                .all(|w| w[0].created_at >= w[1].created_at)
        );
        let all = store.documents(owner, Collection::Bases).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(
            store
                .list(owner, Collection::Solves)
                .await
                .unwrap()
                .is_empty()
        );

        assert!(store.delete(owner, Collection::Bases, a.id).await.unwrap());
        assert!(!store.delete(owner, Collection::Bases, a.id).await.unwrap());
        assert_eq!(
            store.get(owner, Collection::Bases, a.id).await.unwrap(),
            None
        );
        assert_eq!(store.list(owner, Collection::Bases).await.unwrap().len(), 1);
    }

    #[sqlx::test]
    async fn owners_see_only_their_own(pool: PgPool) {
        let store = Store::from_pool(pool.clone());
        let mine = account(&store, "doctor").await;
        let theirs = account(&store, "kaltsit").await;
        let mut doc = Document::new(mine, None, json!({ "v": 1 }));
        store.insert(Collection::Rosters, &doc).await.unwrap();

        let peek = store
            .get(theirs, Collection::Rosters, doc.id)
            .await
            .unwrap();
        assert_eq!(peek, None);
        assert!(
            store
                .list(theirs, Collection::Rosters)
                .await
                .unwrap()
                .is_empty()
        );
        let docs = store.documents(theirs, Collection::Rosters).await.unwrap();
        assert!(docs.is_empty());
        assert!(
            !store
                .delete(theirs, Collection::Rosters, doc.id)
                .await
                .unwrap()
        );
        let mut stolen = doc.clone();
        stolen.owner = theirs;
        stolen.body = json!({ "v": "stolen" });
        assert!(!store.update(Collection::Rosters, &stolen).await.unwrap());

        // Housekeeping sees everyone's, oldest first.
        let other = Document::new(theirs, None, json!({ "v": 2 }));
        store.insert(Collection::Rosters, &other).await.unwrap();
        let ids: Vec<Uuid> = store
            .all_documents(Collection::Rosters)
            .await
            .unwrap()
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, [doc.id, other.id]);

        // An account's documents go with it.
        doc.body = json!({ "v": 3 });
        assert!(store.update(Collection::Rosters, &doc).await.unwrap());
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(mine)
            .execute(&pool)
            .await
            .unwrap();
        let ids: Vec<Uuid> = store
            .all_documents(Collection::Rosters)
            .await
            .unwrap()
            .iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, [other.id]);
    }

    #[sqlx::test]
    async fn update_keeps_created_at_and_never_resurrects(pool: PgPool) {
        let store = Store::from_pool(pool);
        let owner = account(&store, "doctor").await;
        let mut doc = Document::new(owner, None, json!({ "v": 1 }));
        store.insert(Collection::Solves, &doc).await.unwrap();
        let created = doc.created_at;
        doc.body = json!({ "v": 2 });
        doc.name = Some("renamed".into());
        doc.touch();
        assert!(store.update(Collection::Solves, &doc).await.unwrap());
        let back = store
            .get(owner, Collection::Solves, doc.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.created_at, created);

        // An update names its collection, and a deleted document stays
        // deleted.
        assert!(!store.update(Collection::Bases, &doc).await.unwrap());
        store
            .delete(owner, Collection::Solves, doc.id)
            .await
            .unwrap();
        assert!(!store.update(Collection::Solves, &doc).await.unwrap());
        let gone = store.get(owner, Collection::Solves, doc.id).await.unwrap();
        assert_eq!(gone, None);
    }

    #[sqlx::test]
    async fn refuses_newer_schemas(pool: PgPool) {
        let store = Store::from_pool(pool);
        let owner = account(&store, "doctor").await;
        let mut doc = Document::new(owner, None, Value::Null);
        doc.schema_version = SCHEMA_VERSION + 1;
        store.insert(Collection::Rosters, &doc).await.unwrap();
        assert!(matches!(
            store.get(owner, Collection::Rosters, doc.id).await,
            Err(StoreError::NewerSchema { .. })
        ));
        assert!(matches!(
            store.documents(owner, Collection::Rosters).await,
            Err(StoreError::NewerSchema { .. })
        ));
        let mut old = Document::new(owner, None, Value::Null);
        old.schema_version = 0;
        assert!(matches!(
            migrate(old),
            Err(StoreError::Migration { from: 0, .. })
        ));
    }

    #[sqlx::test(migrations = false)]
    async fn the_schema_must_be_migrated(pool: PgPool) {
        let store = Store::from_pool(pool.clone());
        let err = store.check_schema().await.unwrap_err();
        assert!(err.to_string().contains("ak-api migrate"), "{err}");
        MIGRATOR.run(&pool).await.unwrap();
        store.check_schema().await.unwrap();
    }

    #[sqlx::test(migrations = false)]
    async fn documents_without_an_owner_stop_the_owner_migration(pool: PgPool) {
        let mut steps = MIGRATOR
            .iter()
            .filter(|m| !m.migration_type.is_down_migration());
        for m in steps.by_ref().take(2) {
            sqlx::raw_sql(&m.sql).execute(&pool).await.unwrap();
        }
        sqlx::query(
            "INSERT INTO documents (collection, id, schema_version, created_at, updated_at, body)              VALUES ('bases', $1, 1, now(), now(), '{}')",
        )
        .bind(Uuid::new_v4())
        .execute(&pool)
        .await
        .unwrap();
        let owners = steps.next().unwrap();
        assert_eq!(owners.description, "document owners");
        let err = sqlx::raw_sql(&owners.sql).execute(&pool).await.unwrap_err();
        assert!(err.to_string().contains("have no owner"), "{err}");
        let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM documents")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(kept, 1);
    }

    #[test]
    fn times_are_written_to_the_millisecond() {
        let doc = Document::new(Uuid::new_v4(), None, Value::Null);
        assert_eq!(doc.created_at.timestamp_subsec_nanos() % 1_000_000, 0);
        let meta = serde_json::to_value(doc.meta()).unwrap();
        let written = meta["created_at"].as_str().unwrap();
        assert_eq!(written, format_time(doc.created_at));
        assert!(written.ends_with('Z') && written.len() == "2026-01-01T00:00:00.000Z".len());
        let read: DocumentMeta = serde_json::from_value(meta).unwrap();
        assert_eq!(read, doc.meta());
    }
}
