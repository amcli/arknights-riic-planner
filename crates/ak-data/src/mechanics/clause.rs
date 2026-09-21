//! Clause splitting and decomposition.
//!
//! [`preprocess`] harvests structured information from parenthetical notes
//! (stacking rule, caps) and then drops them. [`split`] divides the body
//! into clauses at `;`, `. `, `, and`, `, but`, ` and ` and plain commas,
//! but only when the text after the separator starts like a new clause, so
//! "{K:Caster} and {K:Medic} Operators'" and "if X, Y" survive intact.
//! [`parse_clause`] then peels off leading/trailing conditions and counters
//! and hands the core phrase to the rule tables.

use std::sync::LazyLock;

use ak_domain::*;
use regex::Regex;

use super::Ctx;
use super::rules::{self, CondMod};
use super::value::parse_value;

/// Structured information harvested from parentheticals.
#[derive(Debug, Clone, PartialEq)]
pub struct Notes {
    pub stacking: Stacking,
    pub max_count: Option<f64>,
    pub max_total: Option<f64>,
    /// "(excluding self)" was present.
    pub excluding_self: bool,
}

const V: &str = r"\{[VD]:([^}]*)\}";

fn rx(p: &str) -> Regex {
    Regex::new(&p.replace("<V>", V)).expect("clause regex")
}

static STRONGEST: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)strongest|most effective"));
static CAPS_AT: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)caps? at \{K:(\d+)\}"));
static PAREN_MAX_N: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)\(max (\d+)\)"));
static PAREN_MAX_V: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)\(max <V>\)"));
static TRAIL_MAX_V: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i),? max <V>$"));
static EXCLUDING_SELF: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)\(excluding self\)"));
static PAREN: LazyLock<Regex> = LazyLock::new(|| rx(r"\([^()]*\)"));
static WORD_BRACE: LazyLock<Regex> = LazyLock::new(|| rx(r"(\w)\{"));
static BRACE_WORD: LazyLock<Regex> = LazyLock::new(|| rx(r"\}(\w)"));
static BRACE_BRACE: LazyLock<Regex> = LazyLock::new(|| rx(r"\}\{"));
static COMMA_BRACE: LazyLock<Regex> = LazyLock::new(|| rx(r",\{"));
static SPACE_COMMA: LazyLock<Regex> = LazyLock::new(|| rx(r"\s+([,;.])"));
static MULTI_WS: LazyLock<Regex> = LazyLock::new(|| rx(r"\s+"));

/// Cleans a body and harvests its notes.
pub fn preprocess(body: &str) -> (String, Notes) {
    let stacking = if STRONGEST.is_match(body) {
        Stacking::StrongestOfType
    } else {
        Stacking::Additive
    };
    let max_count = CAPS_AT
        .captures(body)
        .and_then(|c| c[1].parse().ok())
        .or_else(|| PAREN_MAX_N.captures(body).and_then(|c| c[1].parse().ok()));
    let mut max_total = PAREN_MAX_V
        .captures(body)
        .and_then(|c| parse_value(&c[1]).ok())
        .map(|v| v.n);
    let excluding_self = EXCLUDING_SELF.is_match(body);

    let mut s = body.to_owned();
    for _ in 0..3 {
        let next = PAREN.replace_all(&s, "").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s = s.replace("<{T:", "{T:");
    s = s.replace("{K:each}", "each");
    if let Some(c) = TRAIL_MAX_V.captures(&s) {
        if max_total.is_none() {
            max_total = parse_value(&c[1]).ok().map(|v| v.n);
        }
        s = TRAIL_MAX_V.replace(&s, "").into_owned();
    }
    let s = WORD_BRACE.replace_all(&s, "${1} {");
    let s = BRACE_WORD.replace_all(&s, "} ${1}");
    let s = BRACE_BRACE.replace_all(&s, "} {");
    let s = COMMA_BRACE.replace_all(&s, ", {");
    let s = SPACE_COMMA.replace_all(&s, "${1}");
    let s = MULTI_WS.replace_all(&s, " ");
    let s = s.trim().trim_end_matches(['.', ',', ';']).trim().to_owned();
    (
        s,
        Notes {
            stacking,
            max_count,
            max_total,
            excluding_self,
        },
    )
}

static SEP: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i);\s*(?:and\s+|furthermore,?\s+|then\s+)?|\.\s+(?:additionally,?\s+)?|,\s*(?:and|but|plus|with|then|furthermore,?)\s+|\s+(?:and|but|then)\s+|,\s+",
    )
});

static OPENER: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r"(?i)^(?:\+?\{[VD]:|increases?|restores?|reduces?|decreases?|the |all |self |own |morale|order|productivity|capacity|storage|clue|hr |drone|for (?:each|every)|every|each|if |when |otherwise|an additional|additional|a further|further|furthermore|plus|it's easier|that |these |other |operators|working|total |any |provides?|gains?|grants?|adds?|convert|swaps|by another|by an additional|also |increase |trading post|\{K:(?:pure gold|precious metal|battle record|originium|power plant|trading post)|\{T:cc\.(?:g\.|bd|w\.|t\.|tag\.|c\.))",
    )
});

static COND_START: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^(?:if |when |for (?:each|every) |for \{|every |each |otherwise)"));

/// Splits a body into clauses.
pub fn split(body: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut skip_first_comma = COND_START.is_match(body);
    for m in SEP.find_iter(body) {
        if m.start() < start {
            continue;
        }
        let after = &body[m.end()..];
        let plain_comma = m.as_str().trim() == ",";
        if skip_first_comma && plain_comma {
            skip_first_comma = false;
            continue;
        }
        if !OPENER.is_match(after) {
            continue;
        }
        pieces.push(body[start..m.start()].trim().to_owned());
        start = m.end();
        skip_first_comma = COND_START.is_match(after);
    }
    pieces.push(body[start..].trim().to_owned());
    pieces.into_iter().filter(|p| !p.is_empty()).collect()
}

static OTHERWISE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^otherwise,?\s*(.*)$"));
static LEAD_COND: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^(?:if|when) (.+?), (.+)$"));
static WHILE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^(.+?),? while (.+)$"));
static LEAD_COUNT_FOR: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^for (?:each |every )?(.+?), (.+)$"));
static LEAD_COUNT_VERB: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"(?i)^(?:every|each) (.+?) (?:increases|makes|grants|gives|adds|restores) (.+)$")
});
static LEAD_COUNT_COMMA: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^(?:every|each) (.+?), (.+)$"));
static TRAIL_MAX: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"(?i)^(.+?),? up to (?:a maximum of )?<V>(?: productivity| order acquisition efficiency)?$")
});
static RAMP: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"(?i)^(.+?) <V> in the first hour and thereafter <V> per hour, up to <V>$")
});
static RAMP_PER_HOUR: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^(.+?) per hour <V>, up to <V>$"));
static RAMP_EXTEND: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^(?:then )?by another <V> per hour, up to (?:a maximum of )?<V>$"));
static TRAIL_COND: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^(.+?) (?:if|when) (.+)$"));
static TRAIL_COUNT: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)^(.+?) for (?:each |every )?(.+)$"));
static THAT_ROOMS: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^that (?:trading post|factory)'s "));

/// Condition modifiers accumulated for one clause.
#[derive(Debug, Default)]
struct Mods {
    predicate: Option<Predicate>,
    product: Option<ProductType>,
    spec_level: Option<u8>,
    subclass: Option<String>,
}

impl Mods {
    fn with_predicate(p: Predicate) -> Self {
        Mods {
            predicate: Some(p),
            ..Mods::default()
        }
    }

    fn apply(&mut self, m: CondMod) {
        match m {
            CondMod::Predicate(p) => {
                let current = self.predicate.take().unwrap_or(Predicate::Always);
                self.predicate = Some(current.and(p));
            }
            CondMod::Product(p) => self.product = Some(p),
            CondMod::SpecLevel(l) => self.spec_level = Some(l),
            CondMod::Subclass(s) => self.subclass = Some(s),
        }
    }

    fn predicate(&self) -> Predicate {
        self.predicate.clone().unwrap_or(Predicate::Always)
    }
}

/// Parses one clause, appending to `out` (or extending its last clause for
/// ramp continuations such as "then by another 5% per hour, up to 20%").
pub fn parse_clause(text: &str, ctx: &Ctx, out: &mut Vec<Clause>) -> Result<(), String> {
    let text = text.trim().trim_matches(|c| c == ',' || c == '.').trim();
    if text.is_empty() {
        return Ok(());
    }

    if let Some(c) = RAMP_EXTEND.captures(text) {
        let per_hour = parse_value(&c[1])?.any();
        let max = parse_value(&c[2])?.any();
        let last = out
            .last_mut()
            .ok_or("ramp continuation with no prior clause")?;
        let amount = last
            .effect
            .amount_mut()
            .ok_or("ramp continuation on an effect without an amount")?;
        let initial = match *amount {
            Amount::Flat { value } => value,
            _ => return Err("ramp continuation on a non-flat amount".to_owned()),
        };
        *amount = Amount::Ramp {
            initial,
            per_hour,
            max,
        };
        return Ok(());
    }

    let (mut mods, rest) = if let Some(c) = OTHERWISE.captures(text) {
        (
            Mods::with_predicate(Predicate::Unmodeled {
                text: "otherwise".to_owned(),
            }),
            c[1].to_owned(),
        )
    } else if let Some(c) = LEAD_COND.captures(text) {
        let mut mods = Mods::default();
        mods.apply(rules::condition_modifier(&c[1]));
        (mods, c[2].to_owned())
    } else {
        (Mods::default(), text.to_owned())
    };

    if let Some(c) = WHILE.captures(&rest) {
        let a = c[1].to_owned();
        let b = c[2].to_owned();
        parse_one(&a, ctx, &mut mods, out)?;
        return parse_one(&b, ctx, &mut mods, out);
    }
    parse_one(&rest, ctx, &mut mods, out)
}

/// Splits at the first " per " whose tail is not "hour…" (which belongs to
/// the effect phrase, e.g. "Morale consumed per hour").
fn split_per(text: &str) -> Option<(String, String)> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(" per ") {
        let at = from + i;
        let tail = &text[at + 5..];
        if !tail.to_ascii_lowercase().starts_with("hour") {
            return Some((text[..at].to_owned(), tail.to_owned()));
        }
        from = at + 5;
    }
    None
}

fn parse_one(text: &str, ctx: &Ctx, mods: &mut Mods, out: &mut Vec<Clause>) -> Result<(), String> {
    let mut text = text.trim().to_owned();
    let mut counter: Option<(f64, Counter)> = None;
    let mut max_total: Option<f64> = ctx.max_total;
    let mut ramp: Option<(f64, f64, f64)> = None;

    // Leading counter forms.
    if let Some(c) = LEAD_COUNT_FOR.captures(&text) {
        counter = Some(rules::parse_counter(&c[1])?);
        text = c[2].to_owned();
    } else if let Some(c) = LEAD_COUNT_VERB.captures(&text) {
        counter = Some(rules::parse_counter(&c[1])?);
        text = c[2].to_owned();
    } else if let Some(c) = LEAD_COUNT_COMMA.captures(&text) {
        counter = Some(rules::parse_counter(&c[1])?);
        text = c[2].to_owned();
    }

    // Ramps.
    if let Some(c) = RAMP.captures(&text) {
        let initial = parse_value(&c[2])?.any();
        let per_hour = parse_value(&c[3])?.any();
        let max = parse_value(&c[4])?.any();
        ramp = Some((initial, per_hour, max));
        text = format!("{} {{V:{}}}", &c[1], &c[2]);
    } else if let Some(c) = RAMP_PER_HOUR.captures(&text) {
        let per_hour = parse_value(&c[2])?.any();
        let max = parse_value(&c[3])?.any();
        ramp = Some((0.0, per_hour, max));
        text = format!("{} {{V:{}}}", &c[1], &c[2]);
    }

    // Trailing "up to N".
    if let Some(c) = TRAIL_MAX.captures(&text) {
        max_total = Some(parse_value(&c[2])?.any());
        text = c[1].to_owned();
    }

    // A rule that matches the whole remaining phrase wins over decomposition
    // (e.g. "the Morale consumed when producing X is reduced by N", whose
    // "when" is not a condition).
    let prev = out.last().map(|c| &c.effect);
    let whole = rules::match_effect(text.trim(), ctx, prev);
    if whole.is_err() {
        // Trailing condition.
        if let Some(c) = TRAIL_COND.captures(&text) {
            let head = c[1].to_owned();
            mods.apply(rules::condition_modifier(&c[2]));
            text = head;
        }

        // Trailing counter.
        if counter.is_none() {
            if let Some(c) = TRAIL_COUNT.captures(&text) {
                counter = Some(rules::parse_counter(&c[2])?);
                text = c[1].to_owned();
            } else if let Some((head, tail)) = split_per(&text) {
                counter = Some(rules::parse_counter(&tail)?);
                text = head;
            }
        }
    }

    let that_room = THAT_ROOMS.is_match(&text);
    let effects = match whole {
        Ok(effects) => effects,
        Err(_) => rules::match_effect(text.trim(), ctx, prev)?,
    };
    let mut predicate = mods.predicate();

    for mut effect in effects {
        if let Some((step, counter)) = &counter {
            match effect.amount().cloned() {
                Some(Amount::Flat { value }) => {
                    *effect.amount_mut().expect("amount present") = Amount::PerCount {
                        per: value,
                        step: *step,
                        counter: counter.clone(),
                        max_count: ctx.max_count,
                        max_total,
                    };
                }
                Some(_) => return Err("counter applied to a non-flat amount".to_owned()),
                None => {
                    if !matches!(effect, Effect::Unmodeled { .. } | Effect::ClueBias { .. }) {
                        effect = Effect::Unmodeled {
                            summary: format!("{} scaled by {counter:?}", effect.kind_name()),
                        };
                    }
                }
            }
        } else if let Some((initial, per_hour, max)) = ramp {
            if let Some(a) = effect.amount_mut() {
                *a = Amount::Ramp {
                    initial,
                    per_hour,
                    max,
                };
            }
        } else if let Some(max) = max_total
            && let Some(Amount::PerCount { max_total: mt, .. }) = effect.amount_mut()
        {
            *mt = Some(max);
        }
        if let Some(p) = mods.product {
            match &mut effect {
                Effect::Productivity { product: slot, .. }
                | Effect::Capacity { product: slot, .. } => *slot = Some(p),
                Effect::Mood { .. } => {
                    predicate = predicate.and(Predicate::Producing { product: p });
                }
                _ => return Err("product filter on a non-factory effect".to_owned()),
            }
        }
        if let Effect::TrainingSpeed {
            spec_level: sl,
            subclass: sc,
            ..
        } = &mut effect
        {
            if mods.spec_level.is_some() {
                *sl = mods.spec_level;
            }
            if mods.subclass.is_some() {
                *sc = mods.subclass.clone();
            }
        } else if mods.spec_level.is_some() || mods.subclass.is_some() {
            return Err("training filter on a non-training effect".to_owned());
        }
        if that_room && let Predicate::OperatorInRoom { who, .. } = &predicate {
            let target = Scope::RoomOf(who.clone());
            match &mut effect {
                Effect::Productivity { scope, .. }
                | Effect::Capacity { scope, .. }
                | Effect::OrderEfficiency { scope, .. }
                | Effect::OrderLimit { scope, .. } => *scope = target,
                _ => {}
            }
        }
        out.push(Clause {
            when: predicate.clone(),
            effect,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preprocess_harvests_notes() {
        let (s, n) = preprocess(
            "restores {V:+0.1} Morale per hour to all Operators (Only the strongest effect of this type takes place); x (caps at {K:4} Operators, strongest effect of the same type applies).",
        );
        assert_eq!(s, "restores {V:+0.1} Morale per hour to all Operators; x");
        assert_eq!(n.stacking, Stacking::StrongestOfType);
        assert_eq!(n.max_count, Some(4.0));
    }

    #[test]
    fn preprocess_fixes_spacing() {
        let (s, _) = preprocess("consumption by{D:-0.5}; above{K:12},{T:cc.bd_a1}{V:+1}");
        assert_eq!(
            s,
            "consumption by {D:-0.5}; above {K:12}, {T:cc.bd_a1} {V:+1}"
        );
    }

    #[test]
    fn split_respects_conditions_and_lists() {
        assert_eq!(
            split("capacity limit {V:+2} and Morale consumed per hour {V:-0.25}"),
            vec![
                "capacity limit {V:+2}",
                "Morale consumed per hour {V:-0.25}"
            ]
        );
        assert_eq!(
            split("{K:Caster} and {K:Medic} Operators' Specialization training speed {V:+30%}"),
            vec!["{K:Caster} and {K:Medic} Operators' Specialization training speed {V:+30%}"]
        );
        assert_eq!(
            split(
                "if {K:Ines} is assigned to the Reception Room, clue collection speed {V:+10%}; if {K:Hoederer} is assigned to a Trading Post, that Trading Post's order limit {V:+1}"
            ),
            vec![
                "if {K:Ines} is assigned to the Reception Room, clue collection speed {V:+10%}",
                "if {K:Hoederer} is assigned to a Trading Post, that Trading Post's order limit {V:+1}"
            ]
        );
        assert_eq!(
            split(
                "productivity {V:+20%} in the first hour and thereafter {V:+1%} per hour, up to {V:+25%}"
            ),
            vec![
                "productivity {V:+20%} in the first hour and thereafter {V:+1%} per hour, up to {V:+25%}"
            ]
        );
        assert_eq!(
            split(
                "order acquisition efficiency {V:+20%}; if {T:cc.angel} is assigned to the same Trading Post, additionally order acquisition efficiency {V:+25%}"
            ),
            vec![
                "order acquisition efficiency {V:+20%}",
                "if {T:cc.angel} is assigned to the same Trading Post, additionally order acquisition efficiency {V:+25%}"
            ]
        );
    }

    #[test]
    fn per_hour_is_not_a_counter() {
        assert_eq!(split_per("Morale consumed per hour {V:-0.25}"), None);
        assert_eq!(
            split_per("restores {V:+0.1} Morale per hour to all Operators"),
            None
        );
        assert_eq!(
            split_per("+{V:+1%} per Reception Room level"),
            Some(("+{V:+1%}".to_owned(), "Reception Room level".to_owned()))
        );
    }
}
