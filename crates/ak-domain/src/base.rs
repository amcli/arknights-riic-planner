//! The player's base: which rooms exist, at what level, and what each one is
//! doing. This is *configuration*, not simulation state: nothing here
//! changes while the simulator runs.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{FormulaId, GameData, Profession, RoomCategory, RoomType, SubProfessionId};

/// Training Room slot whose operator assists (协助位). Only this operator's
/// skills apply, and only this operator drains morale, and only while a
/// [`TrainingJob`] is running.
pub const TRAINING_ASSISTANT_SLOT: u8 = 0;

/// Training Room slot holding the trainee (训练位). The trainee neither
/// works nor rests: no skills, no morale change.
pub const TRAINING_TRAINEE_SLOT: u8 = 1;

string_id! {
    /// A user-facing room label, e.g. `B101` or `dorm-2`. Unique within a
    /// [`BaseConfig`]; not an upstream identifier.
    RoomId
}

/// What a Trading Post sells. Upstream `tradingOrderDesDict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingStrategy {
    /// Pure Gold → LMD ("Business Strategy", upstream `O_GOLD`).
    #[default]
    Gold,
    /// Originium Shards → Orundum ("Mining Strategy", upstream `O_DIAMOND`).
    OriginiumShard,
}

/// What a Training Room is doing, as far as base skills care. The job
/// describes the trainee, whether or not the trainee is stationed in
/// [`TRAINING_TRAINEE_SLOT`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingJob {
    /// The trainee's class.
    pub profession: Profession,
    /// The trainee's subclass, if a skill needs it.
    #[serde(default)]
    pub subclass: Option<SubProfessionId>,
    /// Specialisation level being trained *to* (1..=3).
    pub spec_level: u8,
}

/// Per-kind settings. Fields that do not apply to the room's kind must be
/// left at their defaults; [`BaseConfig::validate`] rejects anything else.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoomSettings {
    /// Factory: the formula being produced. `None` means idle.
    pub formula: Option<FormulaId>,
    /// Trading Post: what is being sold. `None` means [`TradingStrategy::Gold`].
    pub strategy: Option<TradingStrategy>,
    /// Dormitory: ambience (furniture comfort), `0..=comfort_limit`.
    pub ambience: u32,
    /// Training Room: what is being trained. `None` means idle.
    pub training: Option<TrainingJob>,
}

/// One room in the base.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Room {
    /// Label, unique within the base.
    pub id: RoomId,
    /// Kind.
    pub kind: RoomType,
    /// Upgrade level, 1-based.
    pub level: u8,
    /// Kind-specific settings.
    #[serde(default)]
    pub settings: RoomSettings,
}

impl Room {
    /// A room with default settings.
    pub fn new(id: impl Into<RoomId>, kind: RoomType, level: u8) -> Self {
        Room {
            id: id.into(),
            kind,
            level,
            settings: RoomSettings::default(),
        }
    }

    /// Builder: Factory formula.
    pub fn with_formula(mut self, formula: impl Into<FormulaId>) -> Self {
        self.settings.formula = Some(formula.into());
        self
    }

    /// Builder: Trading Post strategy.
    pub fn with_strategy(mut self, strategy: TradingStrategy) -> Self {
        self.settings.strategy = Some(strategy);
        self
    }

    /// Builder: Dormitory ambience.
    pub fn with_ambience(mut self, ambience: u32) -> Self {
        self.settings.ambience = ambience;
        self
    }

    /// Builder: Training Room job.
    pub fn with_training(mut self, job: TrainingJob) -> Self {
        self.settings.training = Some(job);
        self
    }

    /// How many operators can be stationed here at this level. Zero if the
    /// level is out of range.
    pub fn capacity(&self, data: &GameData) -> u8 {
        data.facility(self.kind)
            .and_then(|f| f.phase(self.level))
            .map_or(0, |p| p.max_stationed)
    }

    /// The Trading Post strategy, defaulting to gold.
    pub fn strategy(&self) -> TradingStrategy {
        self.settings.strategy.unwrap_or_default()
    }
}

/// Why a [`BaseConfig`] is not buildable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseError {
    /// Two rooms share a label.
    DuplicateRoom(RoomId),
    /// Elevators, corridors and the Private Quarters are not rooms operators
    /// work in and may not appear in a base config.
    NotStaffable { room: RoomId, kind: RoomType },
    /// Level is zero or above the kind's maximum.
    BadLevel { room: RoomId, level: u8, max: u8 },
    /// More rooms of a kind than the game allows.
    TooMany {
        kind: RoomType,
        count: usize,
        max: u32,
    },
    /// A base has exactly one Control Center.
    ControlCount(usize),
    /// A setting was given for a room kind it does not apply to.
    SettingNotApplicable { room: RoomId, setting: &'static str },
    /// A Factory formula id is not in the game data.
    UnknownFormula { room: RoomId, formula: FormulaId },
    /// A Factory formula needs a higher room level than this room has.
    FormulaNeedsLevel {
        room: RoomId,
        formula: FormulaId,
        needs: u8,
    },
    /// Dormitory ambience above `comfort_limit`.
    AmbienceTooHigh {
        room: RoomId,
        ambience: u32,
        max: u32,
    },
    /// Training specialisation level outside 1..=3.
    BadSpecLevel { room: RoomId, spec_level: u8 },
    /// Rooms draw more power than the Power Plants supply.
    PowerDeficit { supply: i32, demand: i32 },
    /// More rooms of a category than the layout has slots for.
    NoSlots {
        category: RoomCategory,
        rooms: usize,
        slots: usize,
    },
}

impl fmt::Display for BaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaseError::DuplicateRoom(r) => write!(f, "duplicate room id {r}"),
            BaseError::NotStaffable { room, kind } => {
                write!(f, "room {room}: {kind} is not a staffable room")
            }
            BaseError::BadLevel { room, level, max } => {
                write!(f, "room {room}: level {level} is outside 1..={max}")
            }
            BaseError::TooMany { kind, count, max } => {
                write!(f, "{count} rooms of kind {kind}, but the game allows {max}")
            }
            BaseError::ControlCount(n) => {
                write!(f, "a base has exactly one Control Center, found {n}")
            }
            BaseError::SettingNotApplicable { room, setting } => {
                write!(
                    f,
                    "room {room}: setting `{setting}` does not apply to its kind"
                )
            }
            BaseError::UnknownFormula { room, formula } => {
                write!(f, "room {room}: unknown formula {formula}")
            }
            BaseError::FormulaNeedsLevel {
                room,
                formula,
                needs,
            } => write!(f, "room {room}: formula {formula} needs level {needs}"),
            BaseError::AmbienceTooHigh {
                room,
                ambience,
                max,
            } => write!(f, "room {room}: ambience {ambience} exceeds {max}"),
            BaseError::BadSpecLevel { room, spec_level } => {
                write!(
                    f,
                    "room {room}: specialisation level {spec_level} is outside 1..=3"
                )
            }
            BaseError::PowerDeficit { supply, demand } => {
                write!(
                    f,
                    "rooms draw {demand} power but Power Plants supply {supply}"
                )
            }
            BaseError::NoSlots {
                category,
                rooms,
                slots,
            } => write!(
                f,
                "{rooms} {category} rooms, but the layout has {slots} such slots"
            ),
        }
    }
}

impl std::error::Error for BaseError {}

/// The rooms of a base.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BaseConfig {
    /// Every room, in display order.
    pub rooms: Vec<Room>,
}

impl BaseConfig {
    /// Wraps a room list. Call [`validate`](Self::validate) before use.
    pub fn new(rooms: Vec<Room>) -> Self {
        BaseConfig { rooms }
    }

    /// Checks every invariant against the game data.
    pub fn validate(&self, data: &GameData) -> Result<(), BaseError> {
        let mut seen = std::collections::BTreeSet::new();
        for room in &self.rooms {
            if !seen.insert(&room.id) {
                return Err(BaseError::DuplicateRoom(room.id.clone()));
            }
            if !room.kind.is_staffable() {
                return Err(BaseError::NotStaffable {
                    room: room.id.clone(),
                    kind: room.kind,
                });
            }
            let max = data.facility(room.kind).map_or(0, |f| f.max_level());
            if room.level == 0 || room.level > max {
                return Err(BaseError::BadLevel {
                    room: room.id.clone(),
                    level: room.level,
                    max,
                });
            }
            self.validate_settings(room, data)?;
        }
        for &kind in RoomType::ALL {
            let count = self.count_of(kind);
            if let Some(max) = data.facility(kind).and_then(|f| f.max_count)
                && count > max as usize
            {
                return Err(BaseError::TooMany { kind, count, max });
            }
        }
        let controls = self.count_of(RoomType::Control);
        if controls != 1 {
            return Err(BaseError::ControlCount(controls));
        }
        let (supply, demand) = self.power_balance(data);
        if demand > supply {
            return Err(BaseError::PowerDeficit { supply, demand });
        }
        for &category in RoomCategory::ALL {
            let rooms = self
                .rooms
                .iter()
                .filter(|r| {
                    data.facility(r.kind)
                        .is_some_and(|f| f.category == category)
                })
                .count();
            let slots = data.layout.slots_of(category.slot_category()).count();
            if rooms > slots {
                return Err(BaseError::NoSlots {
                    category,
                    rooms,
                    slots,
                });
            }
        }
        Ok(())
    }

    fn validate_settings(&self, room: &Room, data: &GameData) -> Result<(), BaseError> {
        let s = &room.settings;
        let not_applicable = |setting| BaseError::SettingNotApplicable {
            room: room.id.clone(),
            setting,
        };
        if s.formula.is_some() && room.kind != RoomType::Manufacture {
            return Err(not_applicable("formula"));
        }
        if s.strategy.is_some() && room.kind != RoomType::Trading {
            return Err(not_applicable("strategy"));
        }
        if s.ambience != 0 && room.kind != RoomType::Dormitory {
            return Err(not_applicable("ambience"));
        }
        if s.training.is_some() && room.kind != RoomType::Training {
            return Err(not_applicable("training"));
        }
        if let Some(formula) = &s.formula {
            let Some(f) = data.manufacture_formulas.get(formula) else {
                return Err(BaseError::UnknownFormula {
                    room: room.id.clone(),
                    formula: formula.clone(),
                });
            };
            for req in &f.require_rooms {
                if req.room_type == RoomType::Manufacture && req.level > room.level {
                    return Err(BaseError::FormulaNeedsLevel {
                        room: room.id.clone(),
                        formula: formula.clone(),
                        needs: req.level,
                    });
                }
            }
        }
        if s.ambience > data.constants.comfort_limit {
            return Err(BaseError::AmbienceTooHigh {
                room: room.id.clone(),
                ambience: s.ambience,
                max: data.constants.comfort_limit,
            });
        }
        if let Some(job) = &s.training
            && !(1..=3).contains(&job.spec_level)
        {
            return Err(BaseError::BadSpecLevel {
                room: room.id.clone(),
                spec_level: job.spec_level,
            });
        }
        Ok(())
    }

    /// Power supplied by Power Plants and drawn by every other room, as
    /// `(supply, demand)` from each room's `electricity` at its level.
    pub fn power_balance(&self, data: &GameData) -> (i32, i32) {
        let (mut supply, mut demand) = (0, 0);
        for room in &self.rooms {
            let e = data
                .facility(room.kind)
                .and_then(|f| f.phase(room.level))
                .map_or(0, |p| p.electricity);
            if e >= 0 {
                supply += e;
            } else {
                demand -= e;
            }
        }
        (supply, demand)
    }

    /// Looks up a room by label.
    pub fn room(&self, id: &str) -> Option<&Room> {
        self.rooms.iter().find(|r| r.id.as_str() == id)
    }

    /// Rooms of a kind, in display order.
    pub fn rooms_of(&self, kind: RoomType) -> impl Iterator<Item = &Room> {
        self.rooms.iter().filter(move |r| r.kind == kind)
    }

    /// Number of rooms of a kind.
    pub fn count_of(&self, kind: RoomType) -> usize {
        self.rooms_of(kind).count()
    }

    /// The Control Center, if the config has exactly one.
    pub fn control(&self) -> Option<&Room> {
        let mut it = self.rooms_of(RoomType::Control);
        match (it.next(), it.next()) {
            (Some(r), None) => Some(r),
            _ => None,
        }
    }
}
