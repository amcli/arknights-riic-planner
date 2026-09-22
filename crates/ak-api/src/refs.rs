//! References to stored documents inside request bodies.
//!
//! Any request that carries a `base` or a `roster` (evaluate, simulate,
//! solve) may name a stored one instead, as `base_id` or `roster_id`. The
//! reference is resolved when the request arrives, so a solve job keeps the
//! exact base and roster it ran with even if the stored ones change later;
//! the ids are kept alongside as provenance.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ak_store::Collection;

use crate::AppState;
use crate::error::{ApiError, from_value};
use crate::stored;

/// The stored documents a request named.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    /// The stored base the request used, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_id: Option<String>,
    /// The stored roster the request used, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_id: Option<String>,
}

impl Refs {
    /// True when the request named nothing.
    pub fn is_empty(&self) -> bool {
        self.base_id.is_none() && self.roster_id.is_none()
    }
}

/// Replaces `base_id` and `roster_id` in a request object with the stored
/// documents' contents, then reads the result as `T`.
pub async fn resolve<T: DeserializeOwned>(
    s: &AppState,
    request: Value,
) -> Result<(T, Refs), ApiError> {
    let Value::Object(mut map) = request else {
        return Err(ApiError::bad_request("the request must be a JSON object"));
    };
    let mut refs = Refs::default();
    for (field, collection) in [("base", Collection::Bases), ("roster", Collection::Rosters)] {
        let id_field = format!("{field}_id");
        let Some(id) = map.remove(&id_field) else {
            continue;
        };
        let Value::String(id) = id else {
            return Err(ApiError::bad_request(format!(
                "`{id_field}` must be a string"
            )));
        };
        if map.contains_key(field) {
            return Err(ApiError::bad_request(format!(
                "give `{field}` or `{id_field}`, not both"
            )));
        }
        let Some(doc) = stored::find(s, collection, id.clone()).await? else {
            return Err(ApiError::bad_request(format!("no stored {field} {id}")));
        };
        let Some(content) = doc.body.get(field).cloned() else {
            return Err(ApiError::internal(format!(
                "stored {field} {id} has no `{field}` field"
            )));
        };
        map.insert(field.to_owned(), content);
        match collection {
            Collection::Bases => refs.base_id = Some(id),
            _ => refs.roster_id = Some(id),
        }
    }
    let value = from_value(Value::Object(map))?;
    Ok((value, refs))
}
