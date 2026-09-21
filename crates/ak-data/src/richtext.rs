//! Parser for the inline markup in upstream description strings.
//!
//! Informal grammar:
//!
//! ```text
//! text   := (literal | tagged)*
//! tagged := '<' ('@' | '$') name '>' text '</>'
//! name   := one or more characters other than '<', '>', or whitespace
//! ```
//!
//! The parser is deliberately forgiving because upstream data has typos
//! (one EN description contains `<<$cc.bd_b1>`): anything that is not a
//! well-formed open tag is literal text, unclosed tags are closed at end of
//! input, and an unmatched `</>` is dropped.

use ak_domain::{RichNode, RichTag, RichText};

const CLOSE: &str = "</>";

/// Parses a description string into a [`RichText`] tree. Never fails.
pub fn parse(input: &str) -> RichText {
    let mut stack: Vec<Frame> = vec![Frame::new(None)];
    let mut rest = input;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(CLOSE) {
            close_top(&mut stack);
            rest = after;
            continue;
        }
        if let Some((tag, after)) = open_tag(rest) {
            top(&mut stack).flush();
            stack.push(Frame::new(Some(tag)));
            rest = after;
            continue;
        }
        let ch = rest.chars().next().expect("rest is non-empty");
        top(&mut stack).text.push(ch);
        rest = &rest[ch.len_utf8()..];
    }

    while stack.len() > 1 {
        close_top(&mut stack);
    }
    let mut root = stack.pop().expect("root frame");
    root.flush();
    RichText(root.children)
}

/// Classifies a tag name (with sigil) into a [`RichTag`].
pub fn classify(sigil: char, name: &str) -> RichTag {
    match (sigil, name) {
        ('@', "cc.vup") => RichTag::ValueUp,
        ('@', "cc.vdown") => RichTag::ValueDown,
        ('@', "cc.kw") => RichTag::Keyword,
        ('@', "cc.rem") => RichTag::Reminder,
        ('$', term) => RichTag::Term(term.to_owned()),
        (_, other) => RichTag::Other(other.to_owned()),
    }
}

struct Frame {
    tag: Option<RichTag>,
    children: Vec<RichNode>,
    text: String,
}

impl Frame {
    fn new(tag: Option<RichTag>) -> Self {
        Frame {
            tag,
            children: Vec::new(),
            text: String::new(),
        }
    }

    fn flush(&mut self) {
        if !self.text.is_empty() {
            self.children.push(RichNode::Text {
                text: std::mem::take(&mut self.text),
            });
        }
    }
}

fn top(stack: &mut [Frame]) -> &mut Frame {
    stack.last_mut().expect("stack is never empty")
}

fn close_top(stack: &mut Vec<Frame>) {
    if stack.len() <= 1 {
        // Stray `</>` with nothing open: drop it.
        return;
    }
    let mut frame = stack.pop().expect("len > 1");
    frame.flush();
    let node = RichNode::Tagged {
        tag: frame.tag.expect("non-root frames carry a tag"),
        children: frame.children,
    };
    top(stack).children.push(node);
}

/// If `s` starts with a well-formed open tag, returns the tag and the
/// remainder after `>`.
fn open_tag(s: &str) -> Option<(RichTag, &str)> {
    let body = s.strip_prefix('<')?;
    let sigil = body.chars().next()?;
    if sigil != '@' && sigil != '$' {
        return None;
    }
    let end = body.find('>')?;
    let name = &body[sigil.len_utf8()..end];
    if name.is_empty() || name.contains(['<', ' ', '\t', '\n']) {
        return None;
    }
    Some((classify(sigil, name), &body[end + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> RichNode {
        RichNode::Text { text: s.to_owned() }
    }

    fn tagged(tag: RichTag, children: Vec<RichNode>) -> RichNode {
        RichNode::Tagged { tag, children }
    }

    #[test]
    fn plain_text_is_one_node() {
        assert_eq!(parse("hello"), RichText(vec![text("hello")]));
        assert_eq!(parse(""), RichText(vec![]));
    }

    #[test]
    fn value_up_span() {
        let parsed = parse("productivity <@cc.vup>+15%</>");
        assert_eq!(
            parsed,
            RichText(vec![
                text("productivity "),
                tagged(RichTag::ValueUp, vec![text("+15%")]),
            ])
        );
        assert_eq!(parsed.plain(), "productivity +15%");
        assert_eq!(parsed.values_up(), vec!["+15%"]);
    }

    #[test]
    fn nested_term_and_keyword() {
        let parsed = parse("all <$cc.tag.knight><@cc.kw>Knight</></> Operators");
        assert_eq!(
            parsed,
            RichText(vec![
                text("all "),
                tagged(
                    RichTag::Term("cc.tag.knight".into()),
                    vec![tagged(RichTag::Keyword, vec![text("Knight")])]
                ),
                text(" Operators"),
            ])
        );
        assert_eq!(parsed.plain(), "all Knight Operators");
        assert_eq!(parsed.terms(), vec!["cc.tag.knight"]);
        assert_eq!(
            parsed.find_tagged(|t| *t == RichTag::Keyword),
            vec!["Knight"]
        );
    }

    #[test]
    fn upstream_typo_double_angle_is_tolerated() {
        // Real EN data: `<<$cc.bd_b1>` — the first `<` is literal.
        let parsed = parse("x <<$cc.bd_b1>y</> z");
        assert_eq!(
            parsed,
            RichText(vec![
                text("x <"),
                tagged(RichTag::Term("cc.bd_b1".into()), vec![text("y")]),
                text(" z"),
            ])
        );
        assert_eq!(parsed.plain(), "x <y z");
    }

    #[test]
    fn unclosed_tag_is_closed_at_end() {
        let parsed = parse("a <@cc.vup>b");
        assert_eq!(
            parsed,
            RichText(vec![text("a "), tagged(RichTag::ValueUp, vec![text("b")])])
        );
    }

    #[test]
    fn stray_close_is_dropped() {
        assert_eq!(parse("a</>b"), RichText(vec![text("ab")]));
    }

    #[test]
    fn literal_angle_brackets_survive() {
        assert_eq!(parse("1 < 2 > 0"), RichText(vec![text("1 < 2 > 0")]));
        assert_eq!(parse("<b>bold</b>"), RichText(vec![text("<b>bold</b>")]));
    }

    #[test]
    fn unknown_at_tag_is_kept_as_other() {
        let parsed = parse("<@cc.new>x</>");
        assert_eq!(
            parsed,
            RichText(vec![tagged(
                RichTag::Other("cc.new".into()),
                vec![text("x")]
            )])
        );
    }

    #[test]
    fn multibyte_text_is_preserved() {
        let parsed = parse("生产力<@cc.vup>+15%</>，α");
        assert_eq!(parsed.plain(), "生产力+15%，α");
    }
}
