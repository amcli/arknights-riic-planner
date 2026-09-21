//! The "When this Operator is assigned to …" prefix.
//!
//! Most prefixes only restate the room. A few carry a condition ("assigned
//! to the same Trading Post as Texas"), and Workshop prefixes carry the
//! material filter ("to process elite material").

use std::sync::LazyLock;

use ak_domain::{CostFilter, MaterialFilter, OperatorRef, Predicate, RoomType};
use regex::Regex;

use super::terms;

/// What the prefix contributed.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefix {
    /// Condition from the prefix, `Always` for a plain room prefix.
    pub predicate: Predicate,
    /// Workshop material filter.
    pub material: Option<MaterialFilter>,
    /// Workshop base-cost filter.
    pub base_cost: Option<CostFilter>,
    /// Everything after the prefix.
    pub body: String,
}

const K: &str = r"\{K:([^}]*)\}";
const T: &str = r"\{T:([^}]*)\}";
const V: &str = r"\{[VD]:([^}]*)\}";

fn re(p: &str) -> Regex {
    let p = p.replace("<K>", K).replace("<T>", T).replace("<V>", V);
    Regex::new(&format!(
        r"(?i)^when this operator is assigned {p}[,:]\s*(.*)$"
    ))
    .expect("prefix regex")
}

static WORKSHOP: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"to (?:a|the) workshop to process <K>(?:-type materials?| materials?)?(?: with a base morale cost of \{K:(\d+)\})?",
    )
});
static SAME_TP: LazyLock<Regex> = LazyLock::new(|| re(r"to the same trading post as <K>"));
static WITH_GROUP_CC: LazyLock<Regex> =
    LazyLock::new(|| re(r"together with other <T> operators to the control center"));
static WITH_NAME_CC: LazyLock<Regex> =
    LazyLock::new(|| re(r"together with <K> to the control center"));
static CC_WITH_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"to the control center with <K>"));
static MEETING_WITH: LazyLock<Regex> =
    LazyLock::new(|| re(r"to the reception room together with <K>"));
static FACTORY_AND_TP: LazyLock<Regex> =
    LazyLock::new(|| re(r"to a factory and <K> is in a trading post"));
static CC_MOOD_ABOVE: LazyLock<Regex> =
    LazyLock::new(|| re(r"to the control center and own morale is above \{K:(\d+)\}"));
static TRAIN_HOURS: LazyLock<Regex> =
    LazyLock::new(|| re(r"to the training room to train an operator for <V> hours"));
static PLAIN: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"to (?:be the trainer (?:in|of) the training room|(?:a |the )?(?:factory|power plant|trading post|dormitory|control center|reception room|hr office|workshop))",
    )
});

/// Parses the prefix of a templatized description.
pub fn parse_prefix(template: &str, _room: RoomType) -> Result<Prefix, String> {
    let plain = |predicate: Predicate, body: &str| Prefix {
        predicate,
        material: None,
        base_cost: None,
        body: body.to_owned(),
    };
    let who = |name: &str| OperatorRef::named(name.trim());

    if let Some(c) = WORKSHOP.captures(template) {
        return Ok(Prefix {
            predicate: Predicate::Always,
            material: Some(terms::material_from_keyword(&c[1])),
            base_cost: c
                .get(2)
                .map(|m| m.as_str().parse::<u32>())
                .transpose()
                .map_err(|e| e.to_string())?
                .map(CostFilter::Exactly),
            body: c[3].to_owned(),
        });
    }
    if let Some(c) = SAME_TP.captures(template) {
        return Ok(plain(Predicate::CoworkerIs { who: who(&c[1]) }, &c[2]));
    }
    if let Some(c) = WITH_GROUP_CC.captures(template) {
        let group = terms::group(&c[1])?;
        return Ok(plain(Predicate::CoworkerIn { group }, &c[2]));
    }
    if let Some(c) = WITH_NAME_CC
        .captures(template)
        .or_else(|| CC_WITH_NAME.captures(template))
        .or_else(|| MEETING_WITH.captures(template))
    {
        return Ok(plain(Predicate::CoworkerIs { who: who(&c[1]) }, &c[2]));
    }
    if let Some(c) = FACTORY_AND_TP.captures(template) {
        return Ok(plain(
            Predicate::OperatorInRoom {
                who: who(&c[1]),
                room: RoomType::Trading,
            },
            &c[2],
        ));
    }
    if let Some(c) = CC_MOOD_ABOVE.captures(template) {
        let value: f64 = c[1].parse().map_err(|e| format!("{e}"))?;
        return Ok(plain(Predicate::SelfMoodAbove { value }, &c[2]));
    }
    if let Some(c) = TRAIN_HOURS.captures(template) {
        return Ok(plain(
            Predicate::Unmodeled {
                text: format!("has trained the same operator for {} hours", &c[1]),
            },
            &c[2],
        ));
    }
    if let Some(c) = PLAIN.captures(template) {
        return Ok(plain(Predicate::Always, &c[1]));
    }
    Err(format!(
        "unrecognised prefix: {:?}",
        template.chars().take(80).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ak_domain::ProductType;

    #[test]
    fn plain_rooms() {
        for t in [
            "When this Operator is assigned to a Factory, productivity {V:+15%}",
            "When this Operator is assigned to the Control Center: if x, y",
            "When this Operator is assigned to be the Trainer in the Training Room, x",
            "When this operator is assigned to a Trading post, x",
        ] {
            let p = parse_prefix(t, RoomType::Manufacture).unwrap();
            assert_eq!(p.predicate, Predicate::Always, "{t}");
            assert!(!p.body.starts_with(' '), "{t}");
        }
    }

    #[test]
    fn workshop_material_and_cost() {
        let p = parse_prefix(
            "When this Operator is assigned to the Workshop to process {K:Elite materials} with a base Morale cost of {K:2}, byproduct production chance is increased by {V:+10%}",
            RoomType::Workshop,
        )
        .unwrap();
        assert_eq!(
            p.material,
            Some(MaterialFilter::Product(ProductType::Evolve))
        );
        assert_eq!(p.base_cost, Some(CostFilter::Exactly(2)));
        assert_eq!(
            p.body,
            "byproduct production chance is increased by {V:+10%}"
        );
    }

    #[test]
    fn coworker_prefix() {
        let p = parse_prefix(
            "When this Operator is assigned to the same Trading Post as {K:Texas}, Morale consumed each hour {V:-0.1}",
            RoomType::Trading,
        )
        .unwrap();
        assert_eq!(
            p.predicate,
            Predicate::CoworkerIs {
                who: OperatorRef::named("Texas")
            }
        );
    }

    #[test]
    fn unknown_prefix_fails() {
        assert!(parse_prefix("Something else entirely", RoomType::Control).is_err());
    }
}
