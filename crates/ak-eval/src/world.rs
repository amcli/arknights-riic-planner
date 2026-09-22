//! A resolved, index-based view of "who is where with which skills" for one
//! instant. Rebuilt whenever the assignment changes; cheap.

use std::collections::HashMap;

use ak_domain::*;

use crate::config::{MissingRoster, SimConfig};
use crate::result::{SimError, SimWarning, Warnings};

/// A stationed operator.
#[derive(Debug)]
pub struct OpInfo<'a> {
    /// Upstream id.
    pub id: &'a OperatorId,
    /// Static operator data.
    pub op: &'a Operator,
    /// Promotion state used to pick skill tiers.
    pub promotion: UnlockCond,
    /// Active skill tiers (every room kind; filter by room when applying).
    pub skills: Vec<&'a BaseSkill>,
    /// Index into [`World::rooms`].
    pub room: usize,
    /// Slot index within the room.
    pub slot: u8,
}

/// What a stationed operator is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// In a work area: skills apply, morale drains.
    Working,
    /// In a Dormitory: skills apply, morale recovers.
    Resting,
    /// Stationed but neither: a Training Room trainee, or an assistant with
    /// no training running. No skills, no morale change.
    Idle,
}

/// A room and its occupants.
#[derive(Debug)]
pub struct RoomView<'a> {
    /// The room.
    pub room: &'a Room,
    /// Indexes into [`World::ops`].
    pub occupants: Vec<usize>,
}

/// The resolved base.
#[derive(Debug)]
pub struct World<'a> {
    /// Game data.
    pub data: &'a GameData,
    /// The base.
    pub base: &'a BaseConfig,
    /// Run settings.
    pub config: &'a SimConfig,
    /// Rooms in base order.
    pub rooms: Vec<RoomView<'a>>,
    /// Stationed operators.
    pub ops: Vec<OpInfo<'a>>,
    /// Per room: a Training Room with a job that has not finished. The
    /// simulator clears entries when training completes.
    pub training_active: Vec<bool>,
    op_index: HashMap<&'a str, usize>,
}

impl<'a> World<'a> {
    /// Resolves an assignment against the base, roster and game data.
    pub fn build(
        data: &'a GameData,
        base: &'a BaseConfig,
        assignment: &'a Assignment,
        roster: &'a Roster,
        config: &'a SimConfig,
        warn: &mut Warnings,
    ) -> Result<Self, SimError> {
        let mut rooms = Vec::with_capacity(base.rooms.len());
        let mut ops = Vec::new();
        let mut op_index = HashMap::new();
        for (ri, room) in base.rooms.iter().enumerate() {
            let mut occupants = Vec::new();
            let slots = assignment.slots(room.id.as_str()).unwrap_or(&[]);
            for (si, id) in slots.iter().enumerate() {
                let Some(id) = id else { continue };
                let op = data
                    .operators
                    .get(id.as_str())
                    .ok_or_else(|| SimError::UnknownOperator(id.clone()))?;
                let promotion = match roster.get(id.as_str()) {
                    Some(entry) => entry.promotion,
                    None => match config.missing_roster {
                        MissingRoster::AssumeMaxed => {
                            warn.push(SimWarning::OperatorNotInRoster {
                                operator: id.clone(),
                            });
                            UnlockCond::MAX
                        }
                        MissingRoster::Error => return Err(SimError::MissingRoster(id.clone())),
                    },
                };
                let skills = op
                    .active_buffs(promotion)
                    .into_iter()
                    .filter_map(|b| data.skills.get(b.as_str()))
                    .collect();
                op_index.insert(id.as_str(), ops.len());
                occupants.push(ops.len());
                ops.push(OpInfo {
                    id,
                    op,
                    promotion,
                    skills,
                    room: ri,
                    slot: u8::try_from(si).unwrap_or(u8::MAX),
                });
            }
            rooms.push(RoomView { room, occupants });
        }
        let training_active = base
            .rooms
            .iter()
            .map(|r| r.kind == RoomType::Training && r.settings.training.is_some())
            .collect();
        Ok(World {
            data,
            base,
            config,
            rooms,
            ops,
            training_active,
            op_index,
        })
    }

    /// The room an operator is in.
    pub fn room_of(&self, o: usize) -> &RoomView<'a> {
        &self.rooms[self.ops[o].room]
    }

    /// What an operator is doing.
    pub fn role(&self, o: usize) -> Role {
        let info = &self.ops[o];
        match self.rooms[info.room].room.kind {
            RoomType::Dormitory => Role::Resting,
            RoomType::Training => {
                if info.slot == TRAINING_TRAINEE_SLOT || !self.training_active[info.room] {
                    Role::Idle
                } else {
                    Role::Working
                }
            }
            kind if kind.is_work_area() => Role::Working,
            _ => Role::Idle,
        }
    }

    /// Indexes of every room of a kind.
    pub fn rooms_of(&self, kind: RoomType) -> Vec<usize> {
        self.rooms
            .iter()
            .enumerate()
            .filter(|(_, r)| r.room.kind == kind)
            .map(|(i, _)| i)
            .collect()
    }

    /// Indexes of every operator in rooms of a kind.
    pub fn ops_in_rooms_of(&self, kind: RoomType) -> Vec<usize> {
        self.rooms
            .iter()
            .filter(|r| r.room.kind == kind)
            .flat_map(|r| r.occupants.iter().copied())
            .collect()
    }

    /// Indexes of every operator in a work area.
    pub fn ops_in_work_areas(&self) -> Vec<usize> {
        self.rooms
            .iter()
            .filter(|r| r.room.kind.is_work_area())
            .flat_map(|r| r.occupants.iter().copied())
            .collect()
    }

    /// Finds a referenced operator, by resolved id if the loader resolved
    /// the name, else by display name.
    pub fn find_op(&self, who: &OperatorRef) -> Option<usize> {
        match &who.id {
            Some(id) => self.op_index.get(id.as_str()).copied(),
            None => self.ops.iter().position(|o| o.op.name == who.name),
        }
    }

    /// Index of an operator by id.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.op_index.get(id).copied()
    }

    /// An operator's active skills for the room they are in.
    pub fn skills_in_room(&self, o: usize) -> impl Iterator<Item = &'a BaseSkill> + '_ {
        let kind = self.room_of(o).room.kind;
        self.ops[o]
            .skills
            .iter()
            .copied()
            .filter(move |s| s.room_type == kind)
    }

    /// Maximum morale of an operator.
    pub fn max_mood(&self, o: usize) -> f64 {
        self.ops[o].op.max_mood
    }

    /// Whether an operator belongs to a group. `None` when the group has no
    /// membership definition (a warning is recorded).
    pub fn in_group(
        &self,
        o: usize,
        group: &Group,
        skill: &BuffId,
        warn: &mut Warnings,
    ) -> Option<bool> {
        let op = self.ops[o].op;
        match group {
            Group::Power(p) => Some(op.powers().any(|q| q == p)),
            Group::Profession(p) => Some(op.profession == *p),
            Group::Tag(tag) => match self.config.memberships.tags.get(tag) {
                Some(members) => Some(members.contains(op.id.as_str())),
                None => {
                    warn.push(SimWarning::UnknownTag {
                        skill: skill.clone(),
                        tag: tag.clone(),
                    });
                    None
                }
            },
            Group::Subclass(name) => match self.config.memberships.subclass_id(name) {
                Some(id) => Some(op.sub_profession.as_str() == id),
                None => {
                    warn.push(SimWarning::UnknownSubclassName {
                        skill: skill.clone(),
                        name: name.clone(),
                    });
                    None
                }
            },
        }
    }
}
