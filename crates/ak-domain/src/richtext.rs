//! Rich text as used in upstream skill descriptions.
//!
//! Upstream descriptions carry inline markup such as
//! `productivity <@cc.vup>+15%</>` or
//! `all <$cc.tag.knight><@cc.kw>Knight</></> Operators`. This module holds
//! the parsed tree; the parser itself lives in `ak-data::richtext`.

use serde::{Deserialize, Serialize};

/// A markup tag wrapping a span of text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RichTag {
    /// `<@cc.vup>`: a value that increased (green in-game).
    ValueUp,
    /// `<@cc.vdown>`: a value that decreased (red in-game).
    ValueDown,
    /// `<@cc.kw>`: keyword highlight.
    Keyword,
    /// `<@cc.rem>`: reminder / fine print.
    Reminder,
    /// `<$cc.…>`: a tooltip reference to a glossary term (faction, tag, skill).
    /// The value is the term id without the `$` sigil, e.g. `cc.tag.knight`.
    Term(String),
    /// Any other `<@…>` tag we have not classified. Kept verbatim so nothing
    /// is lost; the schema drift check flags these.
    Other(String),
}

/// One node of a rich-text tree.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RichNode {
    /// Literal text.
    Text {
        /// The text.
        text: String,
    },
    /// A tagged span with nested content.
    Tagged {
        /// The wrapping tag.
        tag: RichTag,
        /// Nested nodes.
        children: Vec<RichNode>,
    },
}

/// A parsed rich-text string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RichText(pub Vec<RichNode>);

impl RichText {
    /// A rich text containing only plain text.
    pub fn text(s: impl Into<String>) -> Self {
        RichText(vec![RichNode::Text { text: s.into() }])
    }

    /// The nodes at the top level.
    pub fn nodes(&self) -> &[RichNode] {
        &self.0
    }

    /// The text with all markup stripped.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for node in &self.0 {
            node.write_plain(&mut out);
        }
        out
    }

    /// Plain text of every tagged span (at any depth) whose tag satisfies
    /// `pred`, in document order.
    pub fn find_tagged(&self, pred: impl Fn(&RichTag) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for node in &self.0 {
            node.collect_tagged(&pred, &mut out);
        }
        out
    }

    /// Text of every `<@cc.vup>` span, e.g. `["+15%"]`.
    pub fn values_up(&self) -> Vec<String> {
        self.find_tagged(|t| *t == RichTag::ValueUp)
    }

    /// Text of every `<@cc.vdown>` span.
    pub fn values_down(&self) -> Vec<String> {
        self.find_tagged(|t| *t == RichTag::ValueDown)
    }

    /// Every glossary term referenced, in document order (may repeat).
    pub fn terms(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.visit_tags(&mut |tag| {
            if let RichTag::Term(term) = tag {
                out.push(term.clone());
            }
        });
        out
    }

    /// Calls `f` for every tag in the tree, in document order.
    pub fn visit_tags(&self, f: &mut dyn FnMut(&RichTag)) {
        for node in &self.0 {
            node.visit_tags(f);
        }
    }
}

impl RichNode {
    fn write_plain(&self, out: &mut String) {
        match self {
            RichNode::Text { text } => out.push_str(text),
            RichNode::Tagged { children, .. } => {
                for child in children {
                    child.write_plain(out);
                }
            }
        }
    }

    fn collect_tagged(&self, pred: &dyn Fn(&RichTag) -> bool, out: &mut Vec<String>) {
        if let RichNode::Tagged { tag, children } = self {
            if pred(tag) {
                let mut text = String::new();
                for child in children {
                    child.write_plain(&mut text);
                }
                out.push(text);
            }
            for child in children {
                child.collect_tagged(pred, out);
            }
        }
    }

    fn visit_tags(&self, f: &mut dyn FnMut(&RichTag)) {
        if let RichNode::Tagged { tag, children } = self {
            f(tag);
            for child in children {
                child.visit_tags(f);
            }
        }
    }
}
