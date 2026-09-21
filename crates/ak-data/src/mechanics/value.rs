//! Numeric values inside `{V:…}` / `{D:…}` placeholders.

use std::sync::LazyLock;

use regex::Regex;

/// A parsed value. `n` carries the sign as written (`-0.25` → `-0.25`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Value {
    /// The number.
    pub n: f64,
    /// Whether it was written with a `%`.
    pub percent: bool,
    /// Whether it was written as "becomes N" (a set, not a delta).
    pub becomes: bool,
}

static VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(becomes\s+)?([+-]?)\s*(\d+(?:\.\d+)?)\s*(%?)(?:\s+levels?)?$")
        .expect("value regex")
});

/// Parses the text inside a value placeholder.
pub fn parse_value(raw: &str) -> Result<Value, String> {
    let c = VALUE_RE
        .captures(raw.trim())
        .ok_or_else(|| format!("unrecognised value {raw:?}"))?;
    let mut n: f64 = c[3].parse().map_err(|e| format!("{raw:?}: {e}"))?;
    if &c[2] == "-" {
        n = -n;
    }
    Ok(Value {
        n,
        percent: !c[4].is_empty(),
        becomes: c.get(1).is_some(),
    })
}

impl Value {
    /// The number, requiring a `%` suffix.
    pub fn pct(self) -> Result<f64, String> {
        if self.percent {
            Ok(self.n)
        } else {
            Err(format!("expected a percentage, got {}", self.n))
        }
    }

    /// The number, requiring no `%` suffix.
    pub fn num(self) -> Result<f64, String> {
        if self.percent {
            Err(format!("expected a plain number, got {}%", self.n))
        } else {
            Ok(self.n)
        }
    }

    /// The number regardless of unit.
    pub fn any(self) -> f64 {
        self.n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forms() {
        assert_eq!(
            parse_value("+15%").unwrap(),
            Value {
                n: 15.0,
                percent: true,
                becomes: false
            }
        );
        assert_eq!(
            parse_value("-0.25").unwrap(),
            Value {
                n: -0.25,
                percent: false,
                becomes: false
            }
        );
        assert_eq!(parse_value("2").unwrap().n, 2.0);
        assert!(parse_value("becomes 2").unwrap().becomes);
        assert_eq!(parse_value("+1 level").unwrap().n, 1.0);
        assert!(parse_value("every").is_err());
        assert!(parse_value("cannot gain").is_err());
    }

    #[test]
    fn unit_checks() {
        assert!(parse_value("+15%").unwrap().num().is_err());
        assert!(parse_value("+2").unwrap().pct().is_err());
        assert_eq!(parse_value("+2").unwrap().num().unwrap(), 2.0);
    }
}
