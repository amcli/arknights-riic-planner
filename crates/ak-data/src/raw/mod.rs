//! Permissive mirrors of the upstream JSON.
//!
//! These structs declare only the fields we consume and tolerate everything
//! else. Field names follow upstream (`camelCase` via serde) so a reader can
//! diff them against the JSON directly. Nothing here is validated beyond
//! JSON type shape; that is [`crate::transform`]'s job. Every list is read
//! through [`list`], because upstreams disagree on how to write an empty one.

pub mod building;
pub mod character;
pub mod team;

use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, Deserialize, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

pub use building::RawBuildingData;
pub use character::{RawCharacter, RawCharacterTable, RawRarity};
pub use team::{RawTeam, RawTeamTable};

/// Reads a list that may be written `{}` when empty: Kengxxiao's dumps
/// write an empty list as `[]`, ArknightsAssets' as `{}`. A non-empty
/// object is still an error.
pub(crate) fn list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct List<T>(PhantomData<T>);

    impl<'de, T: Deserialize<'de>> Visitor<'de> for List<T> {
        type Value = Vec<T>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a list, or {} for an empty one")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element()? {
                items.push(item);
            }
            Ok(items)
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<T>, A::Error> {
            match map.next_key::<IgnoredAny>()? {
                None => Ok(Vec::new()),
                Some(_) => Err(de::Error::invalid_type(de::Unexpected::Map, &self)),
            }
        }
    }

    deserializer.deserialize_any(List(PhantomData))
}

/// The three upstream files, deserialised but not yet validated.
#[derive(Debug, Clone)]
pub struct RawBundle {
    /// `building_data.json`.
    pub building: RawBuildingData,
    /// `character_table.json`.
    pub characters: RawCharacterTable,
    /// `handbook_team_table.json`.
    pub teams: RawTeamTable,
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct Holder {
        #[serde(deserialize_with = "super::list")]
        items: Vec<u8>,
    }

    fn read(json: &str) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::from_str::<Holder>(json).map(|h| h.items)
    }

    #[test]
    fn an_empty_list_may_be_written_as_an_empty_object() {
        assert_eq!(read(r#"{ "items": [1, 2] }"#).unwrap(), [1, 2]);
        assert!(read(r#"{ "items": [] }"#).unwrap().is_empty());
        assert!(read(r#"{ "items": {} }"#).unwrap().is_empty());
        assert!(read(r#"{ "items": { "a": 1 } }"#).is_err());
        assert!(read(r#"{ "items": 3 }"#).is_err());
    }
}
