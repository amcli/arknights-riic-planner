//! Newtyped identifiers. Every upstream string ID gets its own type so an
//! `OperatorId` can never be passed where a `BuffId` is expected.

string_id! {
    /// Upstream character id, e.g. `char_285_medic2`.
    OperatorId
}

string_id! {
    /// Upstream base-skill ("buff") id, e.g. `manu_prod_spd[000]`.
    ///
    /// The bracketed suffix distinguishes tiers of the same skill family;
    /// the part before `[` is the family name.
    BuffId
}

string_id! {
    /// A nation, group, or team id from `handbook_team_table`, e.g. `rhodes`,
    /// `rhine`, `action4`. All three levels share one namespace upstream.
    PowerId
}

string_id! {
    /// Upstream subclass id, e.g. `physician`, `fearless`.
    SubProfessionId
}

string_id! {
    /// A physical slot in the base layout, e.g. `slot_34`.
    SlotId
}

string_id! {
    /// Upstream inventory item id, e.g. `2001` (Drill Battle Record).
    ItemId
}

string_id! {
    /// Upstream production formula id (numeric string).
    FormulaId
}

impl BuffId {
    /// The skill family, i.e. the id with its `[...]` tier suffix removed.
    pub fn family(&self) -> &str {
        match self.as_str().find('[') {
            Some(idx) => &self.as_str()[..idx],
            None => self.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn buff_family_strips_tier_suffix() {
        assert_eq!(BuffId::new("manu_prod_spd[000]").family(), "manu_prod_spd");
        assert_eq!(BuffId::new("plain").family(), "plain");
    }

    #[test]
    fn ids_can_be_looked_up_by_str() {
        let mut map = BTreeMap::new();
        map.insert(OperatorId::new("char_1"), 1);
        assert_eq!(map.get("char_1"), Some(&1));
    }
}
