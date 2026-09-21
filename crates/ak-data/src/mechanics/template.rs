//! Rich text → flat template string.
//!
//! Every markup span becomes a placeholder so the rule regexes can address
//! values and references without caring about display text:
//!
//! | markup | placeholder |
//! |---|---|
//! | `<@cc.vup>+15%</>` | `{V:+15%}` |
//! | `<@cc.vdown>-0.25</>` | `{D:-0.25}` |
//! | `<@cc.kw>Guard</>` | `{K:Guard}` |
//! | `<$cc.g.bs>…</>` (any nesting) | `{T:cc.g.bs}` |
//! | `<@cc.rem>…</>` (outside a term) | `{R:…}` |
//!
//! A keyword that wraps exactly one term (`<@cc.kw><$cc.gvial>Gavial</></>`)
//! collapses to the term. Whitespace is normalised.

use ak_domain::{RichNode, RichTag, RichText};

/// Produces the template string for a description.
pub fn templatize(text: &RichText) -> String {
    let mut out = String::new();
    for node in text.nodes() {
        write_node(node, &mut out);
    }
    collapse_whitespace(&out)
}

fn write_node(node: &RichNode, out: &mut String) {
    match node {
        RichNode::Text { text } => out.push_str(text),
        RichNode::Tagged { tag, children } => match tag {
            RichTag::Term(id) => {
                out.push_str("{T:");
                out.push_str(id);
                out.push('}');
            }
            RichTag::ValueUp => wrap("V", children, out),
            RichTag::ValueDown => wrap("D", children, out),
            RichTag::Keyword => {
                if let [
                    RichNode::Tagged {
                        tag: RichTag::Term(id),
                        ..
                    },
                ] = children.as_slice()
                {
                    out.push_str("{T:");
                    out.push_str(id);
                    out.push('}');
                } else {
                    wrap("K", children, out);
                }
            }
            RichTag::Reminder => wrap("R", children, out),
            RichTag::Other(name) => wrap(&format!("O:{name}"), children, out),
        },
    }
}

fn wrap(kind: &str, children: &[RichNode], out: &mut String) {
    out.push('{');
    out.push_str(kind);
    out.push(':');
    let mut inner = String::new();
    for child in children {
        write_plain(child, &mut inner);
    }
    out.push_str(inner.trim());
    out.push('}');
}

fn write_plain(node: &RichNode, out: &mut String) {
    match node {
        RichNode::Text { text } => out.push_str(text),
        RichNode::Tagged { children, .. } => {
            for child in children {
                write_plain(child, out);
            }
        }
    }
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::richtext::parse;

    #[test]
    fn placeholders() {
        let t = templatize(&parse(
            "When this Operator is assigned to a Factory, productivity <@cc.vup>+15%</>",
        ));
        assert_eq!(
            t,
            "When this Operator is assigned to a Factory, productivity {V:+15%}"
        );
    }

    #[test]
    fn nested_term_collapses() {
        let t = templatize(&parse(
            "all <$cc.tag.knight><@cc.kw>Knight</></> Operators and <@cc.kw><$cc.gvial>Gavial</></>",
        ));
        assert_eq!(t, "all {T:cc.tag.knight} Operators and {T:cc.gvial}");
    }

    #[test]
    fn keyword_and_reminder() {
        let t = templatize(&parse(
            "<@cc.kw>Guard</> x <@cc.rem>note</> <@cc.vdown>-0.25</>",
        ));
        assert_eq!(t, "{K:Guard} x {R:note} {D:-0.25}");
    }
}
