//! Rule tables: effect phrases, counters, conditions, and compound
//! sentences.
//!
//! Every regex is case-insensitive and anchored to the whole phrase. In
//! patterns, `<V>` matches a value placeholder, `<K>` a keyword, `<T>` a
//! term, `<N>` a numeric keyword. Rules are tried in order; the first match
//! wins.

use std::sync::LazyLock;

use ak_domain::*;
use regex::{Captures, Regex};

use super::Ctx;
use super::terms::{self, Term};
use super::value::{Value, parse_value};

type Build = fn(&Captures<'_>, &Ctx, Option<&Effect>) -> Result<Vec<Effect>, String>;

/// One effect-phrase rule.
pub struct Rule {
    re: Regex,
    build: Build,
}

fn expand(p: &str) -> String {
    p.replace("<V>", r"\{[VD]:([^}]*)\}")
        .replace("<K>", r"\{K:([^}]*)\}")
        .replace("<T>", r"\{T:([^}]*)\}")
        .replace("<N>", r"\{K:(\d+)\}")
}

fn re(p: &str) -> Regex {
    Regex::new(&format!("(?i)^{}$", expand(p))).expect("rule regex")
}

macro_rules! rules {
    ($($p:expr => $b:expr),* $(,)?) => {
        vec![$(Rule { re: re($p), build: $b }),*]
    };
}

// ---- capture helpers -------------------------------------------------------

fn s<'a>(c: &'a Captures<'_>, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

fn val(c: &Captures<'_>, i: usize) -> Result<Value, String> {
    parse_value(s(c, i))
}

fn pct(c: &Captures<'_>, i: usize) -> Result<f64, String> {
    val(c, i)?.pct()
}

fn num(c: &Captures<'_>, i: usize) -> Result<f64, String> {
    val(c, i)?.num()
}

fn any(c: &Captures<'_>, i: usize) -> Result<f64, String> {
    Ok(val(c, i)?.any())
}

/// A count written as a value: a number, or the words "every" / "each".
fn cnt(c: &Captures<'_>, i: usize) -> Result<f64, String> {
    let raw = s(c, i).trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("every") || raw.eq_ignore_ascii_case("each") {
        return Ok(1.0);
    }
    raw.parse::<f64>()
        .or_else(|_| parse_value(raw).map(|v| v.n))
        .map_err(|e| format!("count {raw:?}: {e}"))
}

/// Percent for "contribution … {V}" phrases, where "becomes 0" means −100%.
fn scale_pct(c: &Captures<'_>, i: usize) -> Result<f64, String> {
    let v = val(c, i)?;
    if v.becomes { Ok(v.n - 100.0) } else { v.pct() }
}

fn n(c: &Captures<'_>, i: usize) -> Result<u32, String> {
    s(c, i).parse().map_err(|e| format!("{e}"))
}

fn flat(v: f64) -> Amount {
    Amount::flat(v)
}

fn one(e: Effect) -> Result<Vec<Effect>, String> {
    Ok(vec![e])
}

fn product(c: &Captures<'_>, i: usize) -> Result<Option<ProductType>, String> {
    let k = s(c, i);
    if k.is_empty() {
        return Ok(None);
    }
    terms::product_from_keyword(k)
        .map(Some)
        .ok_or_else(|| format!("unknown product keyword {k:?}"))
}

fn profession(k: &str) -> Result<Profession, String> {
    terms::profession_from_keyword(k).ok_or_else(|| format!("unknown class keyword {k:?}"))
}

fn professions(c: &Captures<'_>, first: usize, second: usize) -> Result<Vec<Profession>, String> {
    let mut out = Vec::new();
    for i in [first, second] {
        let k = s(c, i);
        if !k.is_empty() {
            out.push(profession(k)?);
        }
    }
    Ok(out)
}

fn group(c: &Captures<'_>, i: usize) -> Result<Group, String> {
    terms::group(s(c, i))
}

fn resource(c: &Captures<'_>, i: usize) -> Result<String, String> {
    let id = s(c, i);
    match terms::classify(id) {
        Term::Resource(r) => Ok(r),
        Term::GoldLines => Ok("gold_production_lines".to_owned()),
        other => Err(format!("term {id:?} is not a resource ({other:?})")),
    }
}

fn who(name: &str) -> OperatorRef {
    OperatorRef::named(name.trim())
}

fn prod(amount: Amount, product: Option<ProductType>) -> Effect {
    Effect::Productivity {
        amount,
        product,
        scope: Scope::ThisRoom,
    }
}

fn mood(amount: f64, target: MoodTarget) -> Effect {
    Effect::Mood {
        amount: flat(amount),
        target,
    }
}

fn order_eff(v: f64) -> Effect {
    Effect::OrderEfficiency {
        amount: flat(v),
        scope: Scope::ThisRoom,
    }
}

fn order_limit(v: f64) -> Effect {
    Effect::OrderLimit {
        amount: flat(v),
        scope: Scope::ThisRoom,
    }
}

fn training(v: f64, professions: Vec<Profession>) -> Effect {
    Effect::TrainingSpeed {
        amount: flat(v),
        professions,
        subclass: None,
        spec_level: None,
    }
}

fn material(ctx: &Ctx) -> MaterialFilter {
    ctx.material.clone().unwrap_or(MaterialFilter::Any)
}

fn byproduct(v: f64, ctx: &Ctx) -> Effect {
    Effect::ByproductRate {
        amount: flat(v),
        material: material(ctx),
        base_cost: ctx.base_cost,
    }
}

fn ws_cost(change: CostChange, cost: Option<CostFilter>, ctx: &Ctx) -> Effect {
    Effect::WorkshopMoodCost {
        change,
        material: material(ctx),
        cost,
    }
}

fn per_count(per: f64, counter: Counter, ctx: &Ctx) -> Amount {
    Amount::PerCount {
        per,
        step: 1.0,
        counter,
        max_count: ctx.max_count,
        max_total: ctx.max_total,
    }
}

fn inherit(prev: Option<&Effect>, value: f64) -> Result<Vec<Effect>, String> {
    let prev = prev.ok_or("an 'additional' amount with no preceding effect")?;
    let mut e = prev.clone();
    match e.amount_mut() {
        Some(a) => *a = flat(value),
        None => {
            return Err(format!(
                "cannot add an amount to a {} effect",
                prev.kind_name()
            ));
        }
    }
    Ok(vec![e])
}

fn unmodeled(c: &Captures<'_>, _ctx: &Ctx, _prev: Option<&Effect>) -> Result<Vec<Effect>, String> {
    one(Effect::Unmodeled {
        summary: s(c, 0).to_owned(),
    })
}

fn clue_bias(text: &str) -> Effect {
    Effect::ClueBias {
        description: text.to_owned(),
    }
}

fn first_nonempty<'a>(c: &'a Captures<'_>, i: usize, j: usize) -> &'a str {
    let a = s(c, i);
    if a.is_empty() { s(c, j) } else { a }
}

// ---- effect phrases --------------------------------------------------------

static EFFECT: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    rules![
        // ---- Factory: productivity ----
        r"(?:the factory's )?(?:an additional )?(?:<K> (?:formula(?: related)? )?)?productivity <V>" =>
            |c, _, _| one(prod(flat(pct(c, 2)?), product(c, 1)?)),
        r"(?:gains?|add|provides|provides an additional|provides additional|grant) (?:an additional )?<V> productivity(?: towards <K>)?" =>
            |c, _, _| one(prod(flat(pct(c, 1)?), product(c, 2)?)),
        r"<V> productivity(?: towards <K>)?" =>
            |c, _, _| one(prod(flat(pct(c, 1)?), product(c, 2)?)),
        r"have <V> productivity towards <K> and <V> productivity towards <K>" =>
            |c, _, _| Ok(vec![
                prod(flat(pct(c, 1)?), product(c, 2)?),
                prod(flat(pct(c, 3)?), product(c, 4)?),
            ]),
        r"that factory's productivity by <V>" => |c, _, _| one(prod(flat(pct(c, 1)?), None)),
        r"increases the productivity of the factory <K> is assigned to by <V>" =>
            |c, _, _| one(Effect::Productivity {
                amount: flat(pct(c, 2)?),
                product: None,
                scope: Scope::RoomOf(who(s(c, 1))),
            }),
        r"all factories' productivity <V>" =>
            |c, _, _| one(Effect::Productivity {
                amount: flat(pct(c, 1)?),
                product: None,
                scope: Scope::AllRooms(RoomType::Manufacture),
            }),
        r"all <T> operators assigned to factories gain productivity <V>" =>
            |c, ctx, _| one(Effect::Productivity {
                amount: per_count(pct(c, 2)?, ops(group(c, 1)?, CountScope::TargetRoom), ctx),
                product: None,
                scope: Scope::AllRooms(RoomType::Manufacture),
            }),
        r"the productivity contributed by all other operators in that factory <V>" =>
            |c, _, _| one(Effect::ScaleOthersContribution { stat: Stat::Productivity, percent: scale_pct(c, 1)? }),
        r"the morale consumed when producing <K> is reduced by <V>" =>
            |c, _, _| {
                let _ = product(c, 1)?.ok_or("product")?;
                one(mood(num(c, 2)?.abs(), MoodTarget::SelfOnly))
            },
        // ---- Factory: capacity ----
        r"(?:capacity limit|storage capacity)(?: is increased by)? <V>" =>
            |c, _, _| one(Effect::Capacity { amount: flat(num(c, 1)?), product: None, scope: Scope::ThisRoom }),
        r"<V> capacity limit" =>
            |c, _, _| one(Effect::Capacity { amount: flat(num(c, 1)?), product: None, scope: Scope::ThisRoom }),
        // ---- Trading ----
        r"(?:trading post )?(?:an additional )?order acquisition efficiency <V>" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"<V> order acquisition efficiency" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"order efficiency <V>" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"<V> order efficiency" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"increases? order acquisition efficiency by (?:an additional )?<V>" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"(?:additional order acquisition efficiency|order acquisition efficiency additionally|additionally order acquisition efficiency) <V>" =>
            |c, _, _| one(order_eff(pct(c, 1)?)),
        r"(?:provides|grants?) (?:an additional )?<V> order acquisition efficiency" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"other operators working in the trading post have <V> order acquisition efficiency" =>
            |c, ctx, _| one(Effect::OrderEfficiency {
                amount: per_count(pct(c, 1)?, Counter::OtherOperatorsInRoom, ctx),
                scope: Scope::ThisRoom,
            }),
        r"the order acquisition efficiency of this operator by <V>" => |c, _, _| one(order_eff(pct(c, 1)?)),
        r"the order acquisition efficiency contributed by all other operators assigned to that trading post <V>" =>
            |c, _, _| one(Effect::ScaleOthersContribution { stat: Stat::OrderEfficiency, percent: scale_pct(c, 1)? }),
        r"all trading posts' order efficiency <V>" =>
            |c, _, _| one(Effect::OrderEfficiency { amount: flat(pct(c, 1)?), scope: Scope::AllRooms(RoomType::Trading) }),
        r"(?:the |that trading post's )?order limit(?: is increased by)? <V>" => |c, _, _| one(order_limit(num(c, 1)?)),
        r"<V> order limit" => |c, _, _| one(order_limit(num(c, 1)?)),
        r"reduces the order limit by <V>" => |c, _, _| one(order_limit(-num(c, 1)?.abs())),
        r"trading post gains another <V> <T>" =>
            |c, _, _| one(Effect::GainResource { resource: resource(c, 2)?, amount: flat(num(c, 1)?) }),
        r"trading post gains another <N> <T>" =>
            |c, _, _| one(Effect::GainResource { resource: resource(c, 2)?, amount: flat(f64::from(n(c, 1)?)) }),
        // ---- Power ----
        r"(?:increases the )?drone (?:recovery rate|charging speed)(?: is)?(?: by)? <V>" =>
            |c, _, _| one(Effect::DroneRecovery { amount: flat(pct(c, 1)?) }),
        r"<V> drone recovery rate" => |c, _, _| one(Effect::DroneRecovery { amount: flat(pct(c, 1)?) }),
        r"additional charging speed <V>" => |c, _, _| one(Effect::DroneRecovery { amount: flat(pct(c, 1)?) }),
        r"<K> count <V>" => |c, _, _| facility_count(s(c, 1), num(c, 2)?),
        r"\{K:power plants?\} <V>" => |c, _, _| facility_count("power plant", num(c, 1)?),
        // ---- Morale: self ----
        r"(?:self |own )?morale consumed (?:per|each) hour <V>" => |c, _, _| one(mood(-num(c, 1)?, MoodTarget::SelfOnly)),
        r"(?:self |own )?morale consumed (?:per|each) hour is increased by <V>" => |c, _, _| one(mood(-num(c, 1)?.abs(), MoodTarget::SelfOnly)),
        r"(?:self |own )?morale consumed (?:per|each) hour by <V>" => |c, _, _| one(mood(-num(c, 1)?.abs(), MoodTarget::SelfOnly)),
        r"self morale loss per hour <V>" => |c, _, _| one(mood(-num(c, 1)?, MoodTarget::SelfOnly)),
        r"self morale (?:recovered|restored)(?: per hour)? <V>(?: per hour)?" => |c, _, _| one(mood(num(c, 1)?, MoodTarget::SelfOnly)),
        r"(?:reduces the morale consumed each hour by|morale consumed per hour is reduced by|decreases own morale consumption by) <V>" =>
            |c, _, _| one(mood(num(c, 1)?.abs(), MoodTarget::SelfOnly)),
        r"(?:further )?increases own morale restored per hour by <V>" => |c, _, _| one(mood(num(c, 1)?, MoodTarget::SelfOnly)),
        r"self and <K>'s morale recovered per hour <V>" =>
            |c, _, _| Ok(vec![
                mood(num(c, 2)?, MoodTarget::SelfOnly),
                mood(num(c, 2)?, MoodTarget::Named(who(s(c, 1)))),
            ]),
        // ---- Morale: room-wide ----
        r"morale loss of operators in the trading post <V> per hour" => |c, _, _| one(mood(-num(c, 1)?, MoodTarget::AllInRoom)),
        r"morale consumed per hour of all operators in the factory <V>" => |c, _, _| one(mood(-num(c, 1)?, MoodTarget::AllInRoom)),
        r"total morale consumed is increased by <V> per hour" => |c, _, _| one(mood(-num(c, 1)?.abs(), MoodTarget::AllInRoom)),
        r"(?:also )?increases (?:the )?morale (?:recovery per hour )?of all operators in the control center by <V>(?: per hour)?" =>
            |c, _, _| one(mood(num(c, 1)?, MoodTarget::AllInRoom)),
        r"(?:also )?increases (?:the )?morale (?:consumption|consumed per hour) of (?:all )?operators (?:assigned to|in) the control center by <V>" =>
            |c, _, _| one(mood(-num(c, 1)?.abs(), MoodTarget::AllInRoom)),
        r"all operators in dormitories recover <V> morale per hour" => |c, _, _| one(mood(num(c, 1)?, MoodTarget::Rooms(RoomType::Dormitory))),
        r"(?:working )?operators (?:working )?in <T> (?:will )?recover <V> morale per hour" =>
            |c, ctx, prev| match terms::classify(s(c, 1)) {
                Term::RoomSet("room2") => one(mood(num(c, 2)?, MoodTarget::AllWorkAreas)),
                _ => unmodeled(c, ctx, prev),
            },
        // ---- Morale: dormitory ----
        r"restores <V> morale per hour to all operators (?:assigned to|in) that dormitory" => |c, _, _| one(mood(num(c, 1)?, MoodTarget::AllInRoom)),
        r"restores <V> morale per hour to all other operators assigned to that dormitory(?: whose morale is not full)?" =>
            |c, _, _| one(mood(num(c, 1)?, MoodTarget::OthersInRoom)),
        r"restores the morale of all other operators assigned to that dorm(?:itory)? by <V> per hour" =>
            |c, _, _| one(mood(num(c, 1)?, MoodTarget::OthersInRoom)),
        r"restores <V> morale per hour to (?:another|one other) operators? assigned to that dormitory whose morale is not full" =>
            |c, _, _| one(mood(num(c, 1)?, MoodTarget::OneOtherInRoom)),
        r"restores <V> morale per hour distributed evenly to operators assigned to that dormitory whose morale is not full" =>
            |c, _, _| one(mood(num(c, 1)?, MoodTarget::DistributedInRoom)),
        r"morale recovery per hour of all operators in that dormitory <V>" => |c, _, _| one(mood(num(c, 1)?, MoodTarget::AllInRoom)),
        r"restores an additional <V> to operators whose morale is below <V>" =>
            |c, _, _| one(Effect::Unmodeled { summary: format!("restores an additional {} to operators whose morale is below {}", s(c, 1), s(c, 2)) }),
        // ---- Reception ----
        r"(?:increases? )?clue (?:search|collection) speed(?: increases)?(?: by)?(?: an additional)? <V>" =>
            |c, _, _| one(Effect::ClueSpeed { amount: flat(pct(c, 1)?) }),
        r"increases clue collection speed in the reception room by <V>" => |c, _, _| one(Effect::ClueSpeed { amount: flat(pct(c, 1)?) }),
        // ---- Office ----
        r"(?:increases? (?:the )?)?hr contacting speed(?: by)? <V>" => |c, _, _| one(Effect::ContactSpeed { amount: flat(pct(c, 1)?) }),
        r"<V> hr contacting speed" => |c, _, _| one(Effect::ContactSpeed { amount: flat(pct(c, 1)?) }),
        r"(?:adds )?extra contacting speed <V>" => |c, _, _| one(Effect::ContactSpeed { amount: flat(pct(c, 1)?) }),
        // ---- Training ----
        r"(?:<K>(?: and <K>)? )?operators' specialization training speed ?<V>" =>
            |c, _, _| one(training(pct(c, 3)?, professions(c, 1, 2)?)),
        r"increases the specialization training speed of <K> operators by <V>" =>
            |c, _, _| one(training(pct(c, 2)?, vec![profession(s(c, 1))?])),
        r"<V> specialization training speed" => |c, _, _| one(training(pct(c, 1)?, Vec::new())),
        r"that operator's specialization training speed <V>" => |c, _, _| one(training(pct(c, 1)?, Vec::new())),
        // ---- Workshop ----
        r"(?:the )?(?:production rate of byproduct|byproduct production rate|byproduct chance|byproduct production chance)(?: increases| is increased)?(?: by)? <V>" =>
            |c, ctx, _| one(byproduct(pct(c, 1)?, ctx)),
        r"increases the byproduct production rate by <V>" => |c, ctx, _| one(byproduct(pct(c, 1)?, ctx)),
        r"reduces the morale consumed by all (?:corresponding )?formulas that cost <N>(?: morale)?( or more)? by <V>" =>
            |c, ctx, _| one(ws_cost(CostChange::Delta(-num(c, 3)?.abs()), Some(cost_filter(n(c, 1)?, !s(c, 2).is_empty())), ctx)),
        r"decreases the morale consumption of recipes that cost <N> morale or more by <V>" =>
            |c, ctx, _| one(ws_cost(CostChange::Delta(-num(c, 2)?.abs()), Some(CostFilter::AtLeast(n(c, 1)?)), ctx)),
        r"the corresponding formulas will only consume <V> morale" =>
            |c, ctx, _| one(ws_cost(CostChange::Set(num(c, 1)?), None, ctx)),
        r"all formulas that cost <N> morale now <V> morale cost" =>
            |c, ctx, _| one(ws_cost(set_or_delta(val(c, 2)?), Some(CostFilter::Exactly(n(c, 1)?)), ctx)),
        r"recipes with morale cost of <N> have <V> morale cost" =>
            |c, ctx, _| one(ws_cost(set_or_delta(val(c, 2)?), Some(CostFilter::Exactly(n(c, 1)?)), ctx)),
        r"any recipes with morale cost of <N> or higher have their morale cost divided by <V>" =>
            |c, ctx, _| one(ws_cost(CostChange::Divide(num(c, 2)?), Some(CostFilter::AtLeast(n(c, 1)?)), ctx)),
        r"all morale costs <V>" => |c, ctx, _| one(ws_cost(CostChange::Delta(num(c, 1)?), None, ctx)),
        r"increases the morale consumed by all corresponding formulas by <V>" =>
            |c, ctx, _| one(ws_cost(CostChange::Delta(num(c, 1)?.abs()), None, ctx)),
        // ---- Resources ----
        r"<T> ?<V>(?: instead)?" =>
            |c, _, _| one(Effect::GainResource { resource: resource(c, 1)?, amount: flat(any(c, 2)?) }),
        r"(?:gain|provide) <V> <T>" =>
            |c, _, _| one(Effect::GainResource { resource: resource(c, 2)?, amount: flat(any(c, 1)?) }),
        r"<V> <T>" =>
            |c, _, _| one(Effect::GainResource { resource: resource(c, 2)?, amount: flat(any(c, 1)?) }),
        r"(?:every|convert every) <V>(?: points? of| levels? of| bottles? of)? <T> (?:is |are )?(?:converted (?:in)?to|to) <V>(?: points? of)? <T>" =>
            |c, _, _| one(Effect::ConvertResource {
                from: resource(c, 2)?,
                per: any(c, 1)?,
                to: resource(c, 4)?,
                amount: any(c, 3)?,
            }),
        r"every <V> <T> gives <V> specialization training speed" =>
            |c, ctx, _| one(Effect::TrainingSpeed {
                amount: Amount::PerCount {
                    per: pct(c, 3)?,
                    step: any(c, 1)?,
                    counter: Counter::Resource { resource: resource(c, 2)? },
                    max_count: ctx.max_count,
                    max_total: ctx.max_total,
                },
                professions: Vec::new(),
                subclass: None,
                spec_level: None,
            }),
        // ---- Clue bias (qualitative but named) ----
        r"it's easier to obtain the clues of <K>" => |c, _, _| one(clue_bias(&format!("more {} clues", s(c, 1)))),
        r"(?:the )?likelihood of (?:the reception room )?obtaining <K> clues is increased" =>
            |c, _, _| one(clue_bias(&format!("more {} clues", s(c, 1)))),
        r"increases the likelihood of obtaining (?:<K>|<T>) clues" =>
            |c, _, _| one(clue_bias(&format!("more {} clues", first_nonempty(c, 1, 2)))),
        r"increases the likelihood of obtaining clues that are not on the clue board" =>
            |_, _, _| one(clue_bias("more clues not on the board")),
        r"increases the likelihood of obtaining clues that are already on the clue board" =>
            |_, _, _| one(clue_bias("more clues already on the board")),
        r"<K> the likelihood of obtaining clues <K>" =>
            |c, _, _| one(clue_bias(&format!("{} likelihood of clues {}", s(c, 1), s(c, 2)))),
        r"<K> the <K> the other operator in reception" =>
            |_, _, _| one(clue_bias("more clues of the other reception operator's faction")),
        // ---- Inherit the previous effect's kind ("an additional +5%") ----
        r"(?:with )?an additional <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"(?:an additional|additional|another) <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"\+?<V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"(?:restores?|restoration) (?:by )?(?:an additional|another) <V>(?: morale)?(?: to all operators)?" =>
            |c, _, p| inherit(p, any(c, 1)?),
        r"further increases (?:morale restored|this speed) by <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"training speed will be further increased by <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"increase (?:byproduct chance|order acquisition efficiency) by an additional <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"increases extra <V>" => |c, _, p| inherit(p, any(c, 1)?),
        r"(?:with )?a further increase of <V>" => |c, _, p| inherit(p, any(c, 1)?),
        // ---- Recognised but unquantifiable ----
        r"<V> morale recovery from any other source" => unmodeled,
        r"swaps morale with the <K> operator assigned to that dormitory" => unmodeled,
        r"<K> any morale reduction effects from <T> operators assigned to the control center that affect <K>" => unmodeled,
        r"<K> the effects of any operators stationed in that factory that would affect the morale consumption of <K>" => unmodeled,
        r"all <T> and <T> skills in the factory are considered <K> skills" => unmodeled,
        r"a byproduct is guaranteed" => unmodeled,
        r"any byproducts of <K> quality produced will be <K>" => unmodeled,
        r"<T> will be obtained at a fixed rate" => unmodeled,
        r"these orders are <V>" => unmodeled,
        r"<K> traded <V>" => unmodeled,
        r"it will be considered a <K>" => unmodeled,
        r"increase the lmd gained by <K>" => unmodeled,
        r"the chance of getting <K> is <K>" => unmodeled,
        r"this operator's morale will be <K>" => unmodeled,
        r"all <K> orders in that trading post will only trade <N> <K>" => unmodeled,
        r"<K> increases <T> operator recovery in the building by an additional <V>" => unmodeled,
        r"<T> in the base gain <T>" => unmodeled,
        r"<T> in the control center will provide additional morale recovery for operators working in <T>" => unmodeled,
        r"a <K> clue is <K> next" => unmodeled,
        r"increases the productivity of operators in that factory based upon their increased capacity limit" => unmodeled,
        r"operators with <V> or less increased capacity limit gain <V> productivity" => unmodeled,
        r"operators with greater than <V> increased capacity limit gain <V> productivity" => unmodeled,
        r"reduces the lmd cost" => unmodeled,
        r"that operator's next training time <V>" => unmodeled,
        r"<T> is accumulated" => unmodeled,
        r"the next <K> operator's skill specialization training to level <N> is completed immediately" => unmodeled,
        r"all <T> is removed" => unmodeled,
        r"remove all <T> and accumulated <T>" => unmodeled,
        r"<K>" => unmodeled,
    ]
});

fn facility_count(room_keyword: &str, delta: f64) -> Result<Vec<Effect>, String> {
    let room = match room_keyword.trim().to_ascii_lowercase().as_str() {
        "power plant" | "power plants" => RoomType::Power,
        "trading post" => RoomType::Trading,
        "factory" => RoomType::Manufacture,
        other => return Err(format!("unknown facility keyword {other:?}")),
    };
    let delta = delta as i32;
    one(Effect::FacilityCount { room, delta })
}

fn cost_filter(n: u32, or_more: bool) -> CostFilter {
    if or_more {
        CostFilter::AtLeast(n)
    } else {
        CostFilter::Exactly(n)
    }
}

fn set_or_delta(v: Value) -> CostChange {
    if v.becomes {
        CostChange::Set(v.n)
    } else {
        CostChange::Delta(v.n)
    }
}

/// Matches a core effect phrase.
pub fn match_effect(core: &str, ctx: &Ctx, prev: Option<&Effect>) -> Result<Vec<Effect>, String> {
    for rule in EFFECT.iter() {
        if let Some(c) = rule.re.captures(core) {
            return (rule.build)(&c, ctx, prev).map_err(|e| format!("{core:?}: {e}"));
        }
    }
    Err(format!("no effect rule matches {core:?}"))
}

// ---- compound sentences ----------------------------------------------------

type CompoundBuild = fn(&Captures<'_>, &Ctx) -> Result<Vec<Clause>, String>;

struct Compound {
    re: Regex,
    build: CompoundBuild,
}

macro_rules! compounds {
    ($($p:expr => $b:expr),* $(,)?) => {
        vec![$(Compound { re: re($p), build: $b }),*]
    };
}

fn always(effects: Vec<Effect>) -> Vec<Clause> {
    effects
        .into_iter()
        .map(|effect| Clause {
            when: Predicate::Always,
            effect,
        })
        .collect()
}

static COMPOUND: LazyLock<Vec<Compound>> = LazyLock::new(|| {
    compounds![
        r"all <T> operators assigned to trading posts gain order acquisition efficiency <V> and order limit <V>" =>
            |c, ctx| {
                let counter = ops(group(c, 1)?, CountScope::TargetRoom);
                Ok(always(vec![
                    Effect::OrderEfficiency { amount: per_count(pct(c, 2)?, counter.clone(), ctx), scope: Scope::AllRooms(RoomType::Trading) },
                    Effect::OrderLimit { amount: per_count(num(c, 3)?, counter, ctx), scope: Scope::AllRooms(RoomType::Trading) },
                ]))
            },
        r"each <T> operator assigned to factories have <V> productivity towards <K> and <V> productivity towards <K>" =>
            |c, ctx| {
                let counter = ops(group(c, 1)?, CountScope::TargetRoom);
                Ok(always(vec![
                    Effect::Productivity { amount: per_count(pct(c, 2)?, counter.clone(), ctx), product: product(c, 3)?, scope: Scope::AllRooms(RoomType::Manufacture) },
                    Effect::Productivity { amount: per_count(pct(c, 4)?, counter, ctx), product: product(c, 5)?, scope: Scope::AllRooms(RoomType::Manufacture) },
                ]))
            },
        r"each (?:operator from )?<T>(?: operator)? (?:restores <K> and )?increases the morale of all operators in the control center by <V> per hour" =>
            |c, ctx| Ok(always(vec![Effect::Mood {
                amount: per_count(num(c, 3)?, ops(group(c, 1)?, CountScope::SameRoom), ctx),
                target: MoodTarget::AllInRoom,
            }])),
        r"for each <T> operator in the base, <K> productivity <V> and morale consumed per hour <V>" =>
            |c, ctx| {
                let counter = ops(group(c, 1)?, CountScope::Base);
                Ok(always(vec![
                    prod(per_count(pct(c, 3)?, counter.clone(), ctx), product(c, 2)?),
                    Effect::Mood { amount: per_count(-num(c, 4)?, counter, ctx), target: MoodTarget::SelfOnly },
                ]))
            },
        r"operators assigned to the factory increase all capacity limits, add <V> productivity" =>
            |c, ctx| Ok(always(vec![prod(per_count(pct(c, 1)?, Counter::OthersStat { stat: Stat::Capacity }, ctx), None)])),
        r"increases order acquisition efficiency by <V> for every difference of <V> order between the current number of orders and the maximum number of orders" =>
            |c, ctx| Ok(always(vec![Effect::OrderEfficiency {
                amount: Amount::PerCount {
                    per: pct(c, 1)?,
                    step: any(c, 2)?,
                    counter: Counter::Unmodeled { text: "orders below the order limit".to_owned() },
                    max_count: ctx.max_count,
                    max_total: ctx.max_total,
                },
                scope: Scope::ThisRoom,
            }])),
        r"every time that a non-<K> clue is collected, (?:the likelihood of obtaining <K> clues is increased|increases the likelihood of obtaining <K> clues)" =>
            |c, _| Ok(always(vec![clue_bias(&format!("more {} clues after each other clue", s(c, 1)))])),
        r"increases the likelihood of obtaining <K> clues for every newly-obtained clue that is not from <K>" =>
            |c, _| Ok(always(vec![clue_bias(&format!("more {} clues after each other clue", s(c, 1)))])),
    ]
});

/// Tries the compound-sentence rules against a whole body.
pub fn match_compound(body: &str, ctx: &Ctx) -> Result<Option<Vec<Clause>>, String> {
    for rule in COMPOUND.iter() {
        if let Some(c) = rule.re.captures(body) {
            return (rule.build)(&c, ctx)
                .map(Some)
                .map_err(|e| format!("{body:?}: {e}"));
        }
    }
    Ok(None)
}

// ---- counters --------------------------------------------------------------

type CounterBuild = fn(&Captures<'_>) -> Result<(f64, Counter), String>;

struct CounterRule {
    re: Regex,
    build: CounterBuild,
}

macro_rules! counters {
    ($($p:expr => $b:expr),* $(,)?) => {
        vec![$(CounterRule { re: re($p), build: $b }),*]
    };
}

fn ops(group: Group, scope: CountScope) -> Counter {
    Counter::Operators {
        group,
        scope,
        excluding_self: false,
    }
}

/// Like [`ops`], but excludes the skill owner when the matched phrase says
/// "other" ("for every other Rhine Lab Operator in base").
fn ops_in(c: &Captures, group: Group, scope: CountScope) -> Counter {
    let other = c
        .get(0)
        .is_some_and(|m| m.as_str().split_whitespace().any(|w| w == "other"));
    Counter::Operators {
        group,
        scope,
        excluding_self: other,
    }
}

fn room_of(word: &str) -> Result<RoomType, String> {
    match word.trim().to_ascii_lowercase().as_str() {
        "factory" | "factories" => Ok(RoomType::Manufacture),
        "trading post" | "trading posts" => Ok(RoomType::Trading),
        "power plant" | "power plants" => Ok(RoomType::Power),
        "dormitory" | "dormitories" | "dorm" | "dorms" => Ok(RoomType::Dormitory),
        "control center" => Ok(RoomType::Control),
        "reception room" => Ok(RoomType::Meeting),
        "hr office" => Ok(RoomType::Hire),
        "workshop" => Ok(RoomType::Workshop),
        "training room" => Ok(RoomType::Training),
        other => Err(format!("unknown room {other:?}")),
    }
}

static COUNTER: LazyLock<Vec<CounterRule>> = LazyLock::new(|| {
    counters![
        r"<V>(?: points? of| points?| levels? of| bottles? of)? <T>(?: present)?" =>
            |c| Ok((cnt(c, 1)?, counter_for_term(s(c, 2), CountScope::Base)?)),
        r"(?:<N> )?<T>(?: present)?" => |c| Ok((cnt(c, 1)?, counter_for_term(s(c, 2), CountScope::Base)?)),
        r"(?:<V> )?(?:other )?<T> operators? (?:currently )?(?:assigned to|in) (?:the same|this|that) (?:factory|trading post|power plant|dormitory|control center|building)" =>
            |c| Ok((cnt(c, 1)?, ops_in(c, terms::group(s(c, 2))?, CountScope::SameRoom))),
        r"(?:<N> )?(?:other )?<T> operators? in (?:the )?base" =>
            |c| Ok((cnt(c, 1)?, ops_in(c, terms::group(s(c, 2))?, CountScope::Base))),
        r"<T> operators? currently assigned to a non-dormitory facility" =>
            |c| Ok((1.0, ops(terms::group(s(c, 1))?, CountScope::WorkAreas))),
        r"<T> operators? assigned to buildings other than dormitories and activity rooms" =>
            |c| Ok((1.0, ops(terms::group(s(c, 1))?, CountScope::WorkAreas))),
        r"<T> operators? assigned to (?:the )?(factories|trading posts|power plants|control center|dormitories)" =>
            |c| Ok((1.0, ops(terms::group(s(c, 1))?, CountScope::Rooms(room_of(s(c, 2))?)))),
        r"<V> <T> assigned to a power plant" =>
            |c| Ok((cnt(c, 1)?, ops(terms::group(s(c, 2))?, CountScope::Rooms(RoomType::Power)))),
        r"operator from <T>" => |c| Ok((1.0, ops(terms::group(s(c, 1))?, CountScope::SameRoom))),
        r"<T> operators?" => |c| Ok((1.0, ops(terms::group(s(c, 1))?, CountScope::Base))),
        r"<K> operator in the base" => |c| Ok((1.0, ops(Group::Subclass(s(c, 1).to_owned()), CountScope::Base))),
        r"level of (?:each|every) dormitory" => |_| Ok((1.0, Counter::RoomLevels { room: RoomType::Dormitory })),
        r"level of (?:that|the current) (?:trading post|dormitory)" => |_| Ok((1.0, Counter::ThisRoomLevel)),
        r"dormitory level" => |_| Ok((1.0, Counter::ThisRoomLevel)),
        r"reception room level" => |_| Ok((1.0, Counter::RoomLevels { room: RoomType::Meeting })),
        r"(?:<N> )?(?:power plants?|\{K:power plants?\})" => |c| Ok((cnt(c, 1)?, Counter::RoomCount { room: RoomType::Power })),
        r"\{K:trading posts?\}" => |_| Ok((1.0, Counter::RoomCount { room: RoomType::Trading })),
        r"(?:additional )?recruit(?:ment)? slot(?: other than the initial slot)?" => |_| Ok((1.0, Counter::RecruitSlots)),
        r"(?:<N> )?operator in the dormitor(?:y|ies)" => |c| Ok((cnt(c, 1)?, Counter::OperatorsInRooms { room: RoomType::Dormitory })),
        r"additional operator" => |_| Ok((1.0, Counter::OtherOperatorsInRoom)),
        r"operator" => |_| Ok((1.0, Counter::OtherOperatorsInRoom)),
        r"<V> operators in that dormitory" => |c| Ok((cnt(c, 1)?, Counter::OperatorsInRoom)),
        r"<T> in (?:the same|this) factory" =>
            |c| match terms::classify(s(c, 1)) {
                Term::SkillFamily(f) => Ok((1.0, Counter::OperatorsWithSkillFamily { family: f })),
                other => Err(format!("{other:?} is not a skill family")),
            },
        r"<V> (?:order acquisition efficiency|productivity) provided by all other operators (?:assigned to|stationed at) that (?:trading post|factory)" =>
            |c| {
                let stat = if s(c, 0).to_ascii_lowercase().contains("productivity") { Stat::Productivity } else { Stat::OrderEfficiency };
                Ok((cnt(c, 1)?, Counter::OthersStat { stat }))
            },
        r"(?:<V> )?order limit increase provided by (?:all other )?operators (?:assigned to that|in the) trading post" =>
            |c| Ok((cnt(c, 1)?, Counter::OthersStat { stat: Stat::OrderLimit })),
        r"<N> points of <T> on self" =>
            |c| match terms::classify(s(c, 2)) {
                Term::MoodDeficit => Ok((f64::from(n(c, 1)?), Counter::SelfMoodDeficit)),
                other => Err(format!("{other:?} is not the morale-difference term")),
            },
    ]
});

fn counter_for_term(id: &str, default_scope: CountScope) -> Result<Counter, String> {
    match terms::classify(id) {
        Term::Resource(r) => Ok(Counter::Resource { resource: r }),
        Term::GoldLines => Ok(Counter::GoldProductionLines),
        Term::Group(g) => Ok(ops(g, default_scope)),
        other => Err(format!("term {id:?} cannot be counted ({other:?})")),
    }
}

/// Parses a counter phrase ("{T:cc.g.bs} Operator in the Base"). Unknown
/// phrases become [`Counter::Unmodeled`]; only malformed values error.
pub fn parse_counter(text: &str) -> Result<(f64, Counter), String> {
    let text = text.trim().trim_end_matches(',').trim();
    for rule in COUNTER.iter() {
        if let Some(c) = rule.re.captures(text) {
            return (rule.build)(&c).map_err(|e| format!("counter {text:?}: {e}"));
        }
    }
    Ok((
        1.0,
        Counter::Unmodeled {
            text: text.to_owned(),
        },
    ))
}

// ---- conditions ------------------------------------------------------------

/// What a trailing "if/when …" contributes.
#[derive(Debug, Clone, PartialEq)]
pub enum CondMod {
    Predicate(Predicate),
    Product(ProductType),
    SpecLevel(u8),
    Subclass(String),
}

type CondBuild = fn(&Captures<'_>) -> Result<CondMod, String>;

struct CondRule {
    re: Regex,
    build: CondBuild,
}

macro_rules! conds {
    ($($p:expr => $b:expr),* $(,)?) => {
        vec![$(CondRule { re: re($p), build: $b }),*]
    };
}

fn pred(p: Predicate) -> Result<CondMod, String> {
    Ok(CondMod::Predicate(p))
}

fn name_or_term(c: &Captures<'_>, k: usize, t: usize) -> Result<OperatorRef, String> {
    let name = s(c, k);
    if !name.is_empty() {
        return Ok(who(name));
    }
    match terms::classify(s(c, t)) {
        Term::Operator(n) => Ok(who(&n)),
        other => Err(format!("term {:?} is not an operator ({other:?})", s(c, t))),
    }
}

static COND: LazyLock<Vec<CondRule>> = LazyLock::new(|| {
    conds![
        r"producing <K>" => |c| product(c, 1)?.map(CondMod::Product).ok_or_else(|| "product".to_owned()),
        r"training (?:this|the|a) skill to specialization level <V>" => |c| Ok(CondMod::SpecLevel(any(c, 1)? as u8)),
        r"the trainee's job branch is <K>" => |c| Ok(CondMod::Subclass(s(c, 1).to_owned())),
        r"training a <K> operator's skill to specialization level <V>" =>
            |c| pred(Predicate::TrainingProfession { profession: profession(s(c, 1))?, spec_level: Some(any(c, 2)? as u8) }),
        r"(?:<K>|<T>) is assigned to (?:the|a) (?:\{K:)?(control center|reception room|trading post|dormitory|power plant|factory)\}?" =>
            |c| pred(Predicate::OperatorInRoom { who: name_or_term(c, 1, 2)?, room: room_of(s(c, 3))? }),
        r"(?:<K>|<T>) is assigned to the same (?:factory|trading post)" =>
            |c| pred(Predicate::CoworkerIs { who: name_or_term(c, 1, 2)? }),
        r"(?:<K>|<T>) is also assigned to the control center" =>
            |c| pred(Predicate::CoworkerIs { who: name_or_term(c, 1, 2)? }),
        r"in the same (?:factory|trading post) as <K>" => |c| pred(Predicate::CoworkerIs { who: who(s(c, 1)) }),
        r"<K> is in a (trading post|factory|dormitory|power plant)" =>
            |c| pred(Predicate::OperatorInRoom { who: who(s(c, 1)), room: room_of(s(c, 2))? }),
        r"<K> is in the base" => |c| pred(Predicate::OperatorInBase { who: who(s(c, 1)) }),
        r"<K> is assigned to be the trainer in the training room" =>
            |c| pred(Predicate::OperatorInRoom { who: who(s(c, 1)), room: RoomType::Training }),
        r"<K> is assigned to any <T>" => |c| pred(Predicate::OperatorInWorkArea { who: who(s(c, 1)) }),
        r"<K> or <K> are assigned to any <T>" =>
            |c| pred(Predicate::Or { any: vec![
                Predicate::OperatorInWorkArea { who: who(s(c, 1)) },
                Predicate::OperatorInWorkArea { who: who(s(c, 2)) },
            ] }),
        r"(?:a|another) <T> operator is assigned to the same (?:trading post|factory)" =>
            |c| pred(Predicate::CoworkerIn { group: terms::group(s(c, 1))? }),
        r"another <T> operator is assigned to a (power plant|factory|trading post)" =>
            |c| pred(Predicate::GroupInRoom { group: terms::group(s(c, 1))?, room: room_of(s(c, 2))? }),
        r"assigned together with (?:another )?<T> operator" => |c| pred(Predicate::CoworkerIn { group: terms::group(s(c, 1))? }),
        r"assigned together with <K>" => |c| pred(Predicate::CoworkerIs { who: who(s(c, 1)) }),
        r"(?:no other operators are working in the reception room|only this operator is working in the reception room)" =>
            |_| pred(Predicate::AloneInRoom),
        r"in clue exchange" => |_| pred(Predicate::InClueExchange),
        r"(?:self|own) morale is below <N>" => |c| pred(Predicate::SelfMoodBelow { value: f64::from(n(c, 1)?) }),
        r"(?:self|own) morale is above <N>" => |c| pred(Predicate::SelfMoodAbove { value: f64::from(n(c, 1)?) }),
        r"this unit has \{K:full morale\}" => |_| pred(Predicate::SelfMoodFull),
        r"own <T> is greater than <N>" => |c| pred(Predicate::SelfMoodDeficitAbove { value: f64::from(n(c, 2)?) }),
        r"there are <N> or more <T> assigned to \{K:power plants\}" =>
            |c| pred(Predicate::CountAtLeast { counter: ops(terms::group(s(c, 2))?, CountScope::Rooms(RoomType::Power)), n: n(c, 1)? }),
        r"there are no <T> in other \{K:power plants\}" =>
            |c| pred(Predicate::Not { inner: Box::new(Predicate::CountAtLeast { counter: ops(terms::group(s(c, 1))?, CountScope::Rooms(RoomType::Power)), n: 1 }) }),
        r"there are other operators in that dormitory" =>
            |_| pred(Predicate::CountAtLeast { counter: Counter::OtherOperatorsInRoom, n: 1 }),
        r"the target is (?:a )?<K>" => |c| pred(Predicate::TargetIsOperator { who: who(s(c, 1)) }),
        r"the target is (?:a )?<T>(?: operator)?" =>
            |c| match terms::classify(s(c, 1)) {
                Term::Operator(n) => pred(Predicate::TargetIsOperator { who: who(&n) }),
                Term::Group(g) => pred(Predicate::TargetIn { group: g }),
                other => Err(format!("{other:?} cannot be a target")),
            },
        r"an operator is conducting skill specialization in the training room" => |_| pred(Predicate::Always),
    ]
});

/// Parses a condition phrase. Unknown phrases become
/// [`Predicate::Unmodeled`].
pub fn condition_modifier(text: &str) -> CondMod {
    let text = text.trim().trim_end_matches(',').trim();
    for rule in COND.iter() {
        if let Some(c) = rule.re.captures(text) {
            match (rule.build)(&c) {
                Ok(m) => return m,
                Err(e) => {
                    return CondMod::Predicate(Predicate::Unmodeled {
                        text: format!("{text} ({e})"),
                    });
                }
            }
        }
    }
    CondMod::Predicate(Predicate::Unmodeled {
        text: text.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(room: RoomType) -> Ctx {
        Ctx {
            room,
            material: None,
            base_cost: None,
            max_count: None,
            max_total: None,
        }
    }

    #[test]
    fn productivity_rules() {
        let e = match_effect("productivity {V:+15%}", &ctx(RoomType::Manufacture), None).unwrap();
        assert_eq!(e, vec![prod(flat(15.0), None)]);
        let e = match_effect(
            "{K:Precious Metal} formula related productivity {V:+30%}",
            &ctx(RoomType::Manufacture),
            None,
        )
        .unwrap();
        assert_eq!(e, vec![prod(flat(30.0), Some(ProductType::Gold))]);
        assert!(match_effect("productivity {V:+2}", &ctx(RoomType::Manufacture), None).is_err());
    }

    #[test]
    fn mood_sign_conventions() {
        let c = ctx(RoomType::Manufacture);
        assert_eq!(
            match_effect("Morale consumed per hour {V:-0.25}", &c, None).unwrap(),
            vec![mood(0.25, MoodTarget::SelfOnly)]
        );
        assert_eq!(
            match_effect("Morale consumed per hour {D:+0.25}", &c, None).unwrap(),
            vec![mood(-0.25, MoodTarget::SelfOnly)]
        );
        assert_eq!(
            match_effect("self Morale loss per hour {D:-0.5}", &c, None).unwrap(),
            vec![mood(0.5, MoodTarget::SelfOnly)]
        );
        assert_eq!(
            match_effect("Morale consumed per hour is reduced by {V:0.25}", &c, None).unwrap(),
            vec![mood(0.25, MoodTarget::SelfOnly)]
        );
    }

    #[test]
    fn scale_others_becomes_zero() {
        let e = match_effect(
            "the productivity contributed by all other Operators in that Factory {D:becomes 0}",
            &ctx(RoomType::Manufacture),
            None,
        )
        .unwrap();
        assert_eq!(
            e,
            vec![Effect::ScaleOthersContribution {
                stat: Stat::Productivity,
                percent: -100.0
            }]
        );
    }

    #[test]
    fn counters() {
        let (step, c) = parse_counter("{T:cc.g.bs} Operator assigned to Factories").unwrap();
        assert_eq!(step, 1.0);
        assert_eq!(
            c,
            ops(
                Group::Power(PowerId::new("blacksteel")),
                CountScope::Rooms(RoomType::Manufacture)
            )
        );
        let (step, c) = parse_counter("{V:4} {T:cc.bd_b1}").unwrap();
        assert_eq!(step, 4.0);
        assert_eq!(
            c,
            Counter::Resource {
                resource: "worldly_plight".into()
            }
        );
        let (step, c) = parse_counter("{V:every} {T:cc.tag.op} assigned to a Power Plant").unwrap();
        assert_eq!(step, 1.0);
        assert_eq!(
            c,
            ops(Group::Tag("op".into()), CountScope::Rooms(RoomType::Power))
        );
        let (_, c) = parse_counter("level of each Dormitory").unwrap();
        assert_eq!(
            c,
            Counter::RoomLevels {
                room: RoomType::Dormitory
            }
        );
        let (_, c) = parse_counter("something new").unwrap();
        assert!(matches!(c, Counter::Unmodeled { .. }));
    }

    #[test]
    fn conditions() {
        assert_eq!(
            condition_modifier("{K:Warmy} is assigned to the same Factory"),
            CondMod::Predicate(Predicate::CoworkerIs { who: who("Warmy") })
        );
        assert_eq!(
            condition_modifier("{K:Kal'tsit} is assigned to the Control Center"),
            CondMod::Predicate(Predicate::OperatorInRoom {
                who: who("Kal'tsit"),
                room: RoomType::Control
            })
        );
        assert_eq!(
            condition_modifier("producing {K:Battle Records}"),
            CondMod::Product(ProductType::Exp)
        );
        assert_eq!(
            condition_modifier("training this skill to Specialization Level {V:3}"),
            CondMod::SpecLevel(3)
        );
        assert!(matches!(
            condition_modifier("the moon is full"),
            CondMod::Predicate(Predicate::Unmodeled { .. })
        ));
    }

    #[test]
    fn inherit_requires_prev() {
        assert!(match_effect("an additional {V:+5%}", &ctx(RoomType::Trading), None).is_err());
        let prev = order_eff(10.0);
        assert_eq!(
            match_effect(
                "an additional {V:+5%}",
                &ctx(RoomType::Trading),
                Some(&prev)
            )
            .unwrap(),
            vec![order_eff(5.0)]
        );
    }
}
