//! Who is stationed where.
//!
//! [`Assignment`] is a first-class type rather than a bare map so that its
//! invariants hold by construction: every room in the base has exactly as
//! many slots as its level allows, and no operator appears twice. The solver
//! mutates assignments only through [`place`](Assignment::place),
//! [`remove`](Assignment::remove), [`move_to`](Assignment::move_to) and
//! [`swap`](Assignment::swap), so it cannot produce an impossible state.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{BaseConfig, GameData, OperatorId, RoomId};

/// One stationing position: a room and a 0-based index within it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Slot {
    /// The room.
    pub room: RoomId,
    /// Position within the room, `0..capacity`.
    pub index: u8,
}

impl Slot {
    /// Constructs a slot.
    pub fn new(room: impl Into<RoomId>, index: u8) -> Self {
        Slot {
            room: room.into(),
            index,
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.room, self.index)
    }
}

/// Why a placement was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceError {
    /// The assignment has no such room.
    NoSuchRoom(RoomId),
    /// Index beyond the room's capacity.
    IndexOutOfRange { slot: Slot, capacity: usize },
    /// The operator is already stationed elsewhere.
    AlreadyAssigned { operator: OperatorId, at: Slot },
}

impl fmt::Display for PlaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceError::NoSuchRoom(r) => write!(f, "no room {r} in this assignment"),
            PlaceError::IndexOutOfRange { slot, capacity } => {
                write!(f, "slot {slot} is beyond the room's capacity of {capacity}")
            }
            PlaceError::AlreadyAssigned { operator, at } => {
                write!(f, "{operator} is already stationed at {at}")
            }
        }
    }
}

impl std::error::Error for PlaceError {}

/// Why an assignment does not fit a base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentError {
    /// An operator appears in two slots.
    Duplicate(OperatorId),
    /// The base has a room the assignment lacks.
    MissingRoom(RoomId),
    /// The assignment has a room the base lacks.
    UnknownRoom(RoomId),
    /// A room's slot count does not match its capacity at its level.
    WrongCapacity {
        room: RoomId,
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for AssignmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssignmentError::Duplicate(op) => write!(f, "{op} is assigned twice"),
            AssignmentError::MissingRoom(r) => write!(f, "assignment has no entry for room {r}"),
            AssignmentError::UnknownRoom(r) => {
                write!(f, "assignment names room {r}, which the base lacks")
            }
            AssignmentError::WrongCapacity {
                room,
                expected,
                found,
            } => write!(f, "room {room} has {found} slots, expected {expected}"),
        }
    }
}

impl std::error::Error for AssignmentError {}

type SlotMap = BTreeMap<RoomId, Vec<Option<OperatorId>>>;

/// Who is stationed where. See the module docs for the invariants.
///
/// Serialises as a plain `{ room: [operator | null, …] }` map; deserialising
/// re-checks the no-duplicates invariant, and [`check`](Self::check)
/// re-checks the shape against a base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SlotMap", into = "SlotMap")]
pub struct Assignment {
    rooms: SlotMap,
}

impl TryFrom<SlotMap> for Assignment {
    type Error = AssignmentError;

    fn try_from(rooms: SlotMap) -> Result<Self, Self::Error> {
        let a = Assignment { rooms };
        a.check_duplicates()?;
        Ok(a)
    }
}

impl From<Assignment> for SlotMap {
    fn from(a: Assignment) -> Self {
        a.rooms
    }
}

impl Assignment {
    /// An assignment with the given per-room capacities and nobody placed.
    pub fn with_capacities<I, R>(capacities: I) -> Self
    where
        I: IntoIterator<Item = (R, u8)>,
        R: Into<RoomId>,
    {
        Assignment {
            rooms: capacities
                .into_iter()
                .map(|(r, c)| (r.into(), vec![None; usize::from(c)]))
                .collect(),
        }
    }

    /// An empty assignment shaped by a base's rooms and levels.
    pub fn empty(base: &BaseConfig, data: &GameData) -> Self {
        Self::with_capacities(base.rooms.iter().map(|r| (r.id.clone(), r.capacity(data))))
    }

    /// Verifies that this assignment fits the base exactly.
    pub fn check(&self, base: &BaseConfig, data: &GameData) -> Result<(), AssignmentError> {
        self.check_duplicates()?;
        for room in &base.rooms {
            let Some(slots) = self.rooms.get(&room.id) else {
                return Err(AssignmentError::MissingRoom(room.id.clone()));
            };
            let expected = usize::from(room.capacity(data));
            if slots.len() != expected {
                return Err(AssignmentError::WrongCapacity {
                    room: room.id.clone(),
                    expected,
                    found: slots.len(),
                });
            }
        }
        if let Some(extra) = self.rooms.keys().find(|r| base.room(r.as_str()).is_none()) {
            return Err(AssignmentError::UnknownRoom(extra.clone()));
        }
        Ok(())
    }

    fn check_duplicates(&self) -> Result<(), AssignmentError> {
        let mut seen = std::collections::BTreeSet::new();
        for op in self.operators() {
            if !seen.insert(op) {
                return Err(AssignmentError::Duplicate(op.clone()));
            }
        }
        Ok(())
    }

    /// Stations `op` at `slot`, returning whoever was displaced. Fails if
    /// `op` is already stationed at a different slot.
    pub fn place(&mut self, slot: &Slot, op: OperatorId) -> Result<Option<OperatorId>, PlaceError> {
        if let Some(at) = self.locate(op.as_str()) {
            if at == *slot {
                return Ok(None);
            }
            return Err(PlaceError::AlreadyAssigned { operator: op, at });
        }
        let cell = self.cell_mut(slot)?;
        Ok(cell.replace(op))
    }

    /// Clears a slot, returning its occupant. `None` if the slot does not
    /// exist or was empty.
    pub fn remove(&mut self, slot: &Slot) -> Option<OperatorId> {
        self.cell_mut(slot).ok().and_then(Option::take)
    }

    /// Moves `op` to `slot` from wherever they are (or from the bench),
    /// returning whoever was displaced.
    pub fn move_to(
        &mut self,
        op: &OperatorId,
        slot: &Slot,
    ) -> Result<Option<OperatorId>, PlaceError> {
        self.cell_mut(slot)?;
        if let Some(from) = self.locate(op.as_str()) {
            self.remove(&from);
        }
        self.place(slot, op.clone())
    }

    /// Exchanges the occupants of two slots (either may be empty).
    pub fn swap(&mut self, a: &Slot, b: &Slot) -> Result<(), PlaceError> {
        self.cell_mut(a)?;
        self.cell_mut(b)?;
        if a == b {
            return Ok(());
        }
        let x = self.remove(a);
        let y = self.remove(b);
        if let Some(y) = y {
            self.place(a, y)?;
        }
        if let Some(x) = x {
            self.place(b, x)?;
        }
        Ok(())
    }

    /// Where an operator is stationed.
    pub fn locate(&self, op: &str) -> Option<Slot> {
        self.iter()
            .find(|(_, o)| o.as_str() == op)
            .map(|(slot, _)| slot)
    }

    /// The slots of a room, `None` if the room is unknown.
    pub fn slots(&self, room: &str) -> Option<&[Option<OperatorId>]> {
        self.rooms.get(room).map(Vec::as_slice)
    }

    /// Operators stationed in a room.
    pub fn occupants(&self, room: &str) -> impl Iterator<Item = &OperatorId> {
        self.rooms
            .get(room)
            .into_iter()
            .flat_map(|v| v.iter().flatten())
    }

    /// Room labels, sorted.
    pub fn rooms(&self) -> impl Iterator<Item = &RoomId> {
        self.rooms.keys()
    }

    /// Every occupied slot.
    pub fn iter(&self) -> impl Iterator<Item = (Slot, &OperatorId)> {
        self.rooms.iter().flat_map(|(room, slots)| {
            slots.iter().enumerate().filter_map(move |(i, o)| {
                o.as_ref().map(|op| {
                    (
                        Slot {
                            room: room.clone(),
                            index: u8::try_from(i).unwrap_or(u8::MAX),
                        },
                        op,
                    )
                })
            })
        })
    }

    /// Every stationed operator.
    pub fn operators(&self) -> impl Iterator<Item = &OperatorId> {
        self.rooms.values().flat_map(|v| v.iter().flatten())
    }

    /// Number of stationed operators.
    pub fn len(&self) -> usize {
        self.operators().count()
    }

    /// True when nobody is stationed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Empty slots, in room order.
    pub fn vacancies(&self) -> impl Iterator<Item = Slot> + '_ {
        self.rooms.iter().flat_map(|(room, slots)| {
            slots
                .iter()
                .enumerate()
                .filter(|(_, o)| o.is_none())
                .map(move |(i, _)| Slot {
                    room: room.clone(),
                    index: u8::try_from(i).unwrap_or(u8::MAX),
                })
        })
    }

    fn cell_mut(&mut self, slot: &Slot) -> Result<&mut Option<OperatorId>, PlaceError> {
        let Some(slots) = self.rooms.get_mut(&slot.room) else {
            return Err(PlaceError::NoSuchRoom(slot.room.clone()));
        };
        let capacity = slots.len();
        slots
            .get_mut(usize::from(slot.index))
            .ok_or(PlaceError::IndexOutOfRange {
                slot: slot.clone(),
                capacity,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(s: &str) -> OperatorId {
        OperatorId::new(s)
    }

    fn base() -> Assignment {
        Assignment::with_capacities([("B101", 3u8), ("cc", 5), ("dorm", 5)])
    }

    #[test]
    fn place_and_locate() {
        let mut a = base();
        assert_eq!(a.place(&Slot::new("B101", 0), op("x")), Ok(None));
        assert_eq!(a.locate("x"), Some(Slot::new("B101", 0)));
        assert_eq!(a.len(), 1);
        // Same slot again is a no-op.
        assert_eq!(a.place(&Slot::new("B101", 0), op("x")), Ok(None));
        // Elsewhere is refused.
        assert_eq!(
            a.place(&Slot::new("cc", 0), op("x")),
            Err(PlaceError::AlreadyAssigned {
                operator: op("x"),
                at: Slot::new("B101", 0)
            })
        );
    }

    #[test]
    fn displacement_and_bounds() {
        let mut a = base();
        a.place(&Slot::new("B101", 1), op("x")).unwrap();
        assert_eq!(a.place(&Slot::new("B101", 1), op("y")), Ok(Some(op("x"))));
        assert_eq!(a.locate("x"), None);
        assert!(matches!(
            a.place(&Slot::new("B101", 3), op("z")),
            Err(PlaceError::IndexOutOfRange { capacity: 3, .. })
        ));
        assert!(matches!(
            a.place(&Slot::new("nope", 0), op("z")),
            Err(PlaceError::NoSuchRoom(_))
        ));
    }

    #[test]
    fn move_and_swap() {
        let mut a = base();
        a.place(&Slot::new("B101", 0), op("x")).unwrap();
        a.place(&Slot::new("cc", 2), op("y")).unwrap();
        a.move_to(&op("x"), &Slot::new("dorm", 4)).unwrap();
        assert_eq!(a.locate("x"), Some(Slot::new("dorm", 4)));
        a.swap(&Slot::new("dorm", 4), &Slot::new("cc", 2)).unwrap();
        assert_eq!(a.locate("x"), Some(Slot::new("cc", 2)));
        assert_eq!(a.locate("y"), Some(Slot::new("dorm", 4)));
        a.swap(&Slot::new("cc", 2), &Slot::new("B101", 2)).unwrap();
        assert_eq!(a.locate("x"), Some(Slot::new("B101", 2)));
        assert_eq!(a.occupants("cc").count(), 0);
        assert_eq!(a.vacancies().count(), 13 - 2);
    }

    #[test]
    fn serde_rejects_duplicates() {
        let json = r#"{"B101":["x",null,"x"]}"#;
        let err = serde_json::from_str::<Assignment>(json).unwrap_err();
        assert!(err.to_string().contains("assigned twice"), "{err}");

        let mut a = base();
        a.place(&Slot::new("cc", 4), op("q")).unwrap();
        let round: Assignment = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(round, a);
    }
}
