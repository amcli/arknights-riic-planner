//! Closed enums for vocabularies fixed by the game client.
//!
//! Adding a variant here is a deliberate act. Unknown upstream values fail at
//! the ingestion boundary in `ak-data`, never deep inside the evaluator.

str_enum! {
    /// Every kind of room in the base, including the non-staffable ones.
    RoomType {
        /// Control Center.
        Control = "CONTROL",
        /// Power Plant.
        Power = "POWER",
        /// Factory.
        Manufacture = "MANUFACTURE",
        /// Trading Post.
        Trading = "TRADING",
        Dormitory = "DORMITORY",
        /// The Doctor's private quarters. Cosmetic; no base skills target it.
        Private = "PRIVATE",
        Workshop = "WORKSHOP",
        /// Office (upstream calls it `HIRE`).
        Hire = "HIRE",
        /// Training Room.
        Training = "TRAINING",
        /// Reception Room (upstream calls it `MEETING`).
        Meeting = "MEETING",
        Elevator = "ELEVATOR",
        Corridor = "CORRIDOR",
    }
}

impl RoomType {
    /// Rooms that base skills can target. This is exactly the set of
    /// `roomType` values observed on upstream buffs.
    pub const fn has_base_skills(self) -> bool {
        !matches!(
            self,
            RoomType::Private | RoomType::Elevator | RoomType::Corridor
        )
    }

    /// Rooms operators can be stationed in: everything except the Private
    /// Quarters, elevators and corridors.
    pub const fn is_staffable(self) -> bool {
        !matches!(
            self,
            RoomType::Private | RoomType::Elevator | RoomType::Corridor
        )
    }

    /// Staffable rooms other than Dormitories: where operators *work* and
    /// drain morale. Upstream calls these "non-Dormitory facilities".
    pub const fn is_work_area(self) -> bool {
        self.is_staffable() && !matches!(self, RoomType::Dormitory)
    }

    /// Stable English name, independent of the loaded locale.
    pub const fn english_name(self) -> &'static str {
        match self {
            RoomType::Control => "Control Center",
            RoomType::Power => "Power Plant",
            RoomType::Manufacture => "Factory",
            RoomType::Trading => "Trading Post",
            RoomType::Dormitory => "Dormitory",
            RoomType::Private => "Private Quarters",
            RoomType::Workshop => "Workshop",
            RoomType::Hire => "Office",
            RoomType::Training => "Training Room",
            RoomType::Meeting => "Reception Room",
            RoomType::Elevator => "Elevator",
            RoomType::Corridor => "Corridor",
        }
    }
}

str_enum! {
    /// Upstream grouping of room kinds.
    RoomCategory {
        Special = "SPECIAL",
        /// Production facilities (Factory, Trading Post, Power Plant).
        Output = "OUTPUT",
        Function = "FUNCTION",
        Custom = "CUSTOM",
        CustomP = "CUSTOM_P",
    }
}

impl RoomCategory {
    /// The layout slot category that rooms of this category are built in.
    pub const fn slot_category(self) -> SlotCategory {
        match self {
            RoomCategory::Special => SlotCategory::Special,
            RoomCategory::Output => SlotCategory::Output,
            RoomCategory::Function => SlotCategory::Function,
            RoomCategory::Custom => SlotCategory::Custom,
            RoomCategory::CustomP => SlotCategory::CustomP,
        }
    }
}

str_enum! {
    /// Category of a physical layout slot. A superset of [`RoomCategory`]
    /// because elevators and corridors occupy slots but are not rooms.
    SlotCategory {
        Special = "SPECIAL",
        Output = "OUTPUT",
        Function = "FUNCTION",
        Custom = "CUSTOM",
        CustomP = "CUSTOM_P",
        Elevator = "ELEVATOR",
        Corridor = "CORRIDOR",
    }
}

str_enum! {
    /// Upstream classification of a base skill's role.
    BuffCategory {
        Function = "FUNCTION",
        Recovery = "RECOVERY",
        Output = "OUTPUT",
    }
}

str_enum! {
    /// Promotion (elite) level. Ordered: `E0 < E1 < E2`.
    ElitePhase {
        E0 = "PHASE_0",
        E1 = "PHASE_1",
        E2 = "PHASE_2",
    }
}

str_enum! {
    /// Operator rarity. Upstream spells these `TIER_1`..`TIER_6`; older
    /// snapshots used the integers 0..5, which `ak-data` maps here.
    Rarity {
        Tier1 = "TIER_1",
        Tier2 = "TIER_2",
        Tier3 = "TIER_3",
        Tier4 = "TIER_4",
        Tier5 = "TIER_5",
        Tier6 = "TIER_6",
    }
}

impl Rarity {
    /// Star count as shown in-game (1..=6).
    pub const fn stars(self) -> u8 {
        match self {
            Rarity::Tier1 => 1,
            Rarity::Tier2 => 2,
            Rarity::Tier3 => 3,
            Rarity::Tier4 => 4,
            Rarity::Tier5 => 5,
            Rarity::Tier6 => 6,
        }
    }

    /// Maps the legacy zero-based integer encoding (0 = one star).
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Rarity::Tier1),
            1 => Some(Rarity::Tier2),
            2 => Some(Rarity::Tier3),
            3 => Some(Rarity::Tier4),
            4 => Some(Rarity::Tier5),
            5 => Some(Rarity::Tier6),
            _ => None,
        }
    }
}

str_enum! {
    /// Operator class. Variant names use the Global client's names; the
    /// upstream strings are the internal ones.
    Profession {
        Vanguard = "PIONEER",
        Guard = "WARRIOR",
        Defender = "TANK",
        Sniper = "SNIPER",
        Caster = "CASTER",
        Medic = "MEDIC",
        Supporter = "SUPPORT",
        Specialist = "SPECIAL",
    }
}

str_enum! {
    /// Product families a Factory or Workshop can produce. Used both by
    /// production formulas and by skill efficiency-target metadata.
    ProductType {
        /// Pure Gold (`F_GOLD`).
        Gold = "F_GOLD",
        /// Battle Records (`F_EXP`).
        Exp = "F_EXP",
        /// Originium Shards (`F_DIAMOND`).
        OriginiumShard = "F_DIAMOND",
        /// Workshop: building materials.
        Building = "F_BUILDING",
        /// Workshop: elite (promotion) materials.
        Evolve = "F_EVOLVE",
        /// Workshop: skill summaries.
        Skill = "F_SKILL",
        /// Chip / record conversion (`F_ASC`).
        Asc = "F_ASC",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_type_round_trips_upstream_strings() {
        for &rt in RoomType::ALL {
            assert_eq!(rt.as_str().parse::<RoomType>(), Ok(rt));
        }
        assert_eq!(RoomType::ALL.len(), 12);
    }

    #[test]
    fn unknown_value_is_rejected() {
        let err = "GARAGE".parse::<RoomType>().unwrap_err();
        assert_eq!(err.type_name, "RoomType");
        assert_eq!(err.value, "GARAGE");
    }

    #[test]
    fn elite_phase_is_ordered() {
        assert!(ElitePhase::E0 < ElitePhase::E1 && ElitePhase::E1 < ElitePhase::E2);
    }

    #[test]
    fn rarity_legacy_index() {
        assert_eq!(Rarity::from_index(5), Some(Rarity::Tier6));
        assert_eq!(Rarity::from_index(6), None);
        assert_eq!(Rarity::Tier3.stars(), 3);
    }
}
