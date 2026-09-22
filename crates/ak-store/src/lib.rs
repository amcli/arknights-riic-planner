//! Persistence (Layer 6): a document store for rosters, bases and solves.
//!
//! Every stored thing is a [`Document`]: an id, timestamps, an optional
//! name, a `schema_version`, and a JSON body. Bodies are never decomposed
//! into columns; the domain still moves too fast for that. Old bodies are
//! migrated on read ([`migrate`]), never in place.
//!
//! The [`Store`] trait is what the API depends on. The only backend today
//! is [`FileStore`]: a directory per collection, one JSON file per
//! document, each write atomic (temp file, then rename). It needs no
//! database or C toolchain, and a Postgres JSONB backend can replace it
//! behind the same trait when one is available.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The document schema this build writes and the newest it can read.
pub const SCHEMA_VERSION: u32 = 1;

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
    /// Every collection.
    pub const ALL: [Collection; 3] = [Collection::Rosters, Collection::Bases, Collection::Solves];

    /// Directory and URL segment.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// A UUID.
    pub id: String,
    /// The schema the body was written with.
    pub schema_version: u32,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// RFC 3339, UTC.
    pub updated_at: String,
    /// Display name, if any.
    #[serde(default)]
    pub name: Option<String>,
    /// The payload.
    pub body: Value,
}

impl Document {
    /// A new document with a fresh id and the current schema version.
    pub fn new(name: Option<String>, body: Value) -> Self {
        let now = now();
        Document {
            id: uuid::Uuid::new_v4().to_string(),
            schema_version: SCHEMA_VERSION,
            created_at: now.clone(),
            updated_at: now,
            name,
            body,
        }
    }

    /// Bumps `updated_at`.
    pub fn touch(&mut self) {
        self.updated_at = now();
    }

    /// The listing view.
    pub fn meta(&self) -> DocumentMeta {
        DocumentMeta {
            id: self.id.clone(),
            name: self.name.clone(),
            schema_version: self.schema_version,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

/// What a listing shows about a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMeta {
    /// A UUID.
    pub id: String,
    /// Display name, if any.
    pub name: Option<String>,
    /// The schema the body was written with.
    pub schema_version: u32,
    /// RFC 3339, UTC.
    pub created_at: String,
    /// RFC 3339, UTC.
    pub updated_at: String,
}

/// The current time as the store writes it: RFC 3339, UTC, milliseconds.
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Why a store operation failed.
#[derive(Debug)]
pub enum StoreError {
    /// The id is not one this store could have issued.
    BadId(String),
    /// File system failure.
    Io { path: PathBuf, source: io::Error },
    /// A stored file is not a document.
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    /// The document was written by a newer build.
    NewerSchema {
        id: String,
        version: u32,
        supported: u32,
    },
    /// No migration path from the document's version.
    Migration {
        id: String,
        from: u32,
        reason: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::BadId(id) => write!(f, "invalid document id {id:?}"),
            StoreError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            StoreError::Json { path, source } => {
                write!(f, "{}: not a document: {source}", path.display())
            }
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

/// A document store.
pub trait Store: Send + Sync {
    /// Writes a document, replacing any with the same id.
    fn put(&self, collection: Collection, doc: &Document) -> Result<(), StoreError>;

    /// Reads a document, migrated to the current schema.
    fn get(&self, collection: Collection, id: &str) -> Result<Option<Document>, StoreError>;

    /// Lists a collection, newest first.
    fn list(&self, collection: Collection) -> Result<Vec<DocumentMeta>, StoreError>;

    /// Removes a document. Returns whether one was there.
    fn delete(&self, collection: Collection, id: &str) -> Result<bool, StoreError>;
}

/// A directory of JSON files.
#[derive(Debug, Clone)]
pub struct FileStore {
    root: PathBuf,
}

impl FileStore {
    /// Opens (creating if needed) a store rooted at `root`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        for c in Collection::ALL {
            let dir = root.join(c.as_str());
            fs::create_dir_all(&dir).map_err(|source| StoreError::Io { path: dir, source })?;
        }
        Ok(FileStore { root })
    }

    /// Where the files live.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, collection: Collection, id: &str) -> Result<PathBuf, StoreError> {
        let valid = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !valid {
            return Err(StoreError::BadId(id.to_owned()));
        }
        Ok(self
            .root
            .join(collection.as_str())
            .join(format!("{id}.json")))
    }
}

impl Store for FileStore {
    fn put(&self, collection: Collection, doc: &Document) -> Result<(), StoreError> {
        let path = self.path(collection, &doc.id)?;
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_vec_pretty(doc).map_err(|source| StoreError::Json {
            path: path.clone(),
            source,
        })?;
        fs::write(&tmp, text).map_err(|source| StoreError::Io {
            path: tmp.clone(),
            source,
        })?;
        fs::rename(&tmp, &path).map_err(|source| StoreError::Io { path, source })
    }

    fn get(&self, collection: Collection, id: &str) -> Result<Option<Document>, StoreError> {
        let path = self.path(collection, id)?;
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Io { path, source }),
        };
        let doc: Document =
            serde_json::from_slice(&bytes).map_err(|source| StoreError::Json { path, source })?;
        migrate(doc).map(Some)
    }

    fn list(&self, collection: Collection) -> Result<Vec<DocumentMeta>, StoreError> {
        let dir = self.root.join(collection.as_str());
        let entries = fs::read_dir(&dir).map_err(|source| StoreError::Io {
            path: dir.clone(),
            source,
        })?;
        let mut metas = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::Io {
                path: dir.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let bytes = fs::read(&path).map_err(|source| StoreError::Io {
                path: path.clone(),
                source,
            })?;
            let doc: Document = serde_json::from_slice(&bytes)
                .map_err(|source| StoreError::Json { path, source })?;
            metas.push(doc.meta());
        }
        metas.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(metas)
    }

    fn delete(&self, collection: Collection, id: &str) -> Result<bool, StoreError> {
        let path = self.path(collection, id)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StoreError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> FileStore {
        let dir = std::env::temp_dir().join(format!("ak-store-test-{}", uuid::Uuid::new_v4()));
        FileStore::open(dir).unwrap()
    }

    #[test]
    fn put_get_list_delete_round_trip() {
        let store = temp_store();
        let a = Document::new(Some("first".into()), serde_json::json!({ "x": 1 }));
        let b = Document::new(None, serde_json::json!([1, 2, 3]));
        store.put(Collection::Bases, &a).unwrap();
        store.put(Collection::Bases, &b).unwrap();

        let back = store.get(Collection::Bases, &a.id).unwrap().unwrap();
        assert_eq!(back, a);
        assert_eq!(store.get(Collection::Rosters, &a.id).unwrap(), None);

        let listed = store.list(Collection::Bases).unwrap();
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
        assert!(store.list(Collection::Solves).unwrap().is_empty());

        assert!(store.delete(Collection::Bases, &a.id).unwrap());
        assert!(!store.delete(Collection::Bases, &a.id).unwrap());
        assert_eq!(store.get(Collection::Bases, &a.id).unwrap(), None);
        assert_eq!(store.list(Collection::Bases).unwrap().len(), 1);
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn overwrite_is_atomic_and_keeps_created_at() {
        let store = temp_store();
        let mut doc = Document::new(None, serde_json::json!({ "v": 1 }));
        store.put(Collection::Solves, &doc).unwrap();
        let created = doc.created_at.clone();
        doc.body = serde_json::json!({ "v": 2 });
        doc.touch();
        store.put(Collection::Solves, &doc).unwrap();
        let back = store.get(Collection::Solves, &doc.id).unwrap().unwrap();
        assert_eq!(back.body, serde_json::json!({ "v": 2 }));
        assert_eq!(back.created_at, created);
        let dir = store.root().join("solves");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = fs::remove_dir_all(store.root());
    }

    #[test]
    fn rejects_bad_ids_and_newer_schemas() {
        let store = temp_store();
        assert!(matches!(
            store.get(Collection::Bases, "../etc/passwd"),
            Err(StoreError::BadId(_))
        ));
        assert!(matches!(
            store.get(Collection::Bases, ""),
            Err(StoreError::BadId(_))
        ));
        let mut doc = Document::new(None, Value::Null);
        doc.schema_version = SCHEMA_VERSION + 1;
        store.put(Collection::Rosters, &doc).unwrap();
        assert!(matches!(
            store.get(Collection::Rosters, &doc.id),
            Err(StoreError::NewerSchema { .. })
        ));
        let mut old = Document::new(None, Value::Null);
        old.schema_version = 0;
        assert!(matches!(
            migrate(old),
            Err(StoreError::Migration { from: 0, .. })
        ));
        let _ = fs::remove_dir_all(store.root());
    }
}
