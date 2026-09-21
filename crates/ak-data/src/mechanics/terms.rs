//! Classification of upstream glossary terms (`<$cc.…>`) and keywords.
//!
//! Term ids are stable across locales; the display text is not. This table
//! is the reverse-engineered meaning of each id observed in the pinned
//! snapshot. Unknown ids classify as [`Term::Unknown`] and fail parsing, so
//! a new term surfaces as an unparsed skill rather than a silent guess.

use ak_domain::{Group, PowerId, ProductType, Profession};

/// What a `{T:…}` reference denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// A set of operators (faction, tag, …).
    Group(Group),
    /// A named accumulator resource, by our snake_case name.
    Resource(String),
    /// "Pure Gold Production Line" (a Factory producing gold).
    GoldLines,
    /// A base-skill family, by upstream term suffix (`manu1`…).
    SkillFamily(String),
    /// A specific operator by display name.
    Operator(String),
    /// "Morale difference" (deficit below maximum).
    MoodDeficit,
    /// A set of rooms: `room1` (certain facilities), `room2` (other
    /// buildings), `room3` (work areas).
    RoomSet(&'static str),
    /// A tooltip-only note with no mechanical content.
    Note,
    /// Not in the table.
    Unknown,
}

/// Classifies a term id such as `cc.g.bs`.
pub fn classify(id: &str) -> Term {
    let power = |p: &str| Term::Group(Group::Power(PowerId::new(p)));
    let tag = |t: &str| Term::Group(Group::Tag(t.to_owned()));
    let res = |r: &str| Term::Resource(r.to_owned());
    match id {
        // Nations / groups / teams (mapped to handbook_team_table ids).
        "cc.g.bs" => power("blacksteel"),
        "cc.g.karlan" => power("karlan"),
        "cc.g.lda" => power("lee"),
        "cc.g.lgd" => power("lgd"),
        "cc.g.ussg" => power("student"),
        "cc.g.R6" => power("rainbow"),
        "cc.g.sui" => power("sui"),
        "cc.g.abyssal" => power("abyssal"),
        "cc.g.glasgow" => power("glasgow"),
        "cc.g.laterano" => power("laterano"),
        "cc.g.rh" => power("rhine"),
        "cc.g.psk" => power("pinus"),
        "cc.g.sm" => power("sami"),
        "cc.g.siracusa" => power("siracusa"),
        "cc.tag.dungeon" => power("laios"),
        // Tags with no table behind them.
        "cc.g.sp" => tag("alternate"),
        "cc.g.A1" => tag("a1"),
        "cc.g.Defence" => tag("defence"),
        "cc.g.Attack" => tag("attack"),
        "cc.tag.knight" => tag("knight"),
        "cc.tag.mh" => tag("mh"),
        "cc.tag.op" => tag("op"),
        "cc.tag.durin" => tag("durin"),
        // Accumulator resources.
        "cc.bd_b1" => res("worldly_plight"),
        "cc.bd_a1" => res("perception_information"),
        "cc.bd_A" => res("chain_of_thought"),
        "cc.bd_B" => res("soundless_resonance"),
        "cc.bd_C" => res("witchcraft_crystal"),
        "cc.bd_malist" => res("engineering_robot"),
        "cc.bd_felyne" => res("felvine"),
        "cc.bd_ash" => res("intelligence_reserve"),
        "cc.bd_tachanka" => res("ursus_specialty_beverage"),
        "cc.bd_dungeon" => res("monster_meal"),
        "cc.bd_a1_a1" => res("memory_fragments"),
        "cc.bd_a1_a2" => res("dreamland"),
        "cc.bd_a1_a3" => res("measure"),
        "cc.t.accmuguard1" => res("martial_arts"),
        "cc.w.ncdeer1" => res("causality"),
        "cc.w.ncdeer2" => res("karma"),
        // Misc.
        "cc.t.flow_gold" => Term::GoldLines,
        "cc.sk.manu1" | "cc.sk.manu2" | "cc.sk.manu3" | "cc.sk.manu4" => {
            Term::SkillFamily(id.trim_start_matches("cc.sk.").to_owned())
        }
        "cc.angel" => Term::Operator("Exusiai".to_owned()),
        "cc.gvial" => Term::Operator("Gavial".to_owned()),
        "cc.bd.costdrop" => Term::MoodDeficit,
        "cc.c.room1" => Term::RoomSet("room1"),
        "cc.c.room2" => Term::RoomSet("room2"),
        "cc.c.room3" => Term::RoomSet("room3"),
        "cc.tra.pepe" | "cc.m.var1" | "cc.t.strong2" | "cc.c.skill" | "cc.c.sui2_1"
        | "cc.c.abyssal2_1" | "cc.c.abyssal2_2" | "cc.c.abyssal2_3" => Term::Note,
        _ => Term::Unknown,
    }
}

/// The group a term denotes, or an error naming the term.
pub fn group(id: &str) -> Result<Group, String> {
    match classify(id) {
        Term::Group(g) => Ok(g),
        other => Err(format!("term {id:?} is not a group ({other:?})")),
    }
}

/// The resource a term denotes, or an error naming the term.
pub fn resource(id: &str) -> Result<String, String> {
    match classify(id) {
        Term::Resource(r) => Ok(r),
        other => Err(format!("term {id:?} is not a resource ({other:?})")),
    }
}

/// Factory product named by a keyword such as `Precious Metal`.
pub fn product_from_keyword(k: &str) -> Option<ProductType> {
    let k = k.trim().to_ascii_lowercase();
    match k.as_str() {
        "precious metal" | "precious metals" | "pure gold" => Some(ProductType::Gold),
        "battle record" | "battle records" => Some(ProductType::Exp),
        "originium" => Some(ProductType::OriginiumShard),
        _ => None,
    }
}

/// Operator class named by a keyword such as `Guard`.
pub fn profession_from_keyword(k: &str) -> Option<Profession> {
    match k.trim().to_ascii_lowercase().as_str() {
        "vanguard" => Some(Profession::Vanguard),
        "guard" => Some(Profession::Guard),
        "defender" => Some(Profession::Defender),
        "sniper" => Some(Profession::Sniper),
        "caster" => Some(Profession::Caster),
        "medic" => Some(Profession::Medic),
        "supporter" => Some(Profession::Supporter),
        "specialist" => Some(Profession::Specialist),
        _ => None,
    }
}

/// Workshop material family named by a keyword such as `elite material`.
pub fn material_from_keyword(k: &str) -> ak_domain::MaterialFilter {
    use ak_domain::MaterialFilter as M;
    let lower = k.trim().to_ascii_lowercase();
    match lower.as_str() {
        "any material" => M::Any,
        "elite material" | "elite materials" => M::Product(ProductType::Evolve),
        "skill summaries" | "skill summary" => M::Product(ProductType::Skill),
        "building material" | "building materials" => M::Product(ProductType::Building),
        "chips" | "chip" => M::Product(ProductType::Asc),
        _ => M::Named(k.trim().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_terms() {
        assert_eq!(
            classify("cc.g.bs"),
            Term::Group(Group::Power(PowerId::new("blacksteel")))
        );
        assert_eq!(
            classify("cc.tag.knight"),
            Term::Group(Group::Tag("knight".into()))
        );
        assert_eq!(
            classify("cc.bd_b1"),
            Term::Resource("worldly_plight".into())
        );
        assert_eq!(classify("cc.t.flow_gold"), Term::GoldLines);
        assert_eq!(classify("cc.nope"), Term::Unknown);
    }

    #[test]
    fn keywords() {
        assert_eq!(
            product_from_keyword("Precious Metal"),
            Some(ProductType::Gold)
        );
        assert_eq!(
            product_from_keyword("Battle Records"),
            Some(ProductType::Exp)
        );
        assert_eq!(
            profession_from_keyword("Supporter"),
            Some(Profession::Supporter)
        );
        assert_eq!(
            material_from_keyword("Elite materials"),
            ak_domain::MaterialFilter::Product(ProductType::Evolve)
        );
        assert_eq!(
            material_from_keyword("Device"),
            ak_domain::MaterialFilter::Named("Device".into())
        );
    }
}
