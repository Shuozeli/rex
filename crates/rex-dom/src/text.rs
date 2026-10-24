//! Normative text semantics (engine spec §8.1 STRING and §8.2 TEXT).

use std::collections::HashSet;

use ego_tree::{NodeId, NodeRef};
use scraper::{ElementRef, Node};
use unicode_normalization::UnicodeNormalization;

use crate::{Budget, BudgetExceeded};

/// Subtrees whose text never contributes.
const SKIP_TAGS: &[&str] = &["script", "style", "noscript", "template"];

/// Elements that start and end a paragraph in block text.
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "aside",
    "nav",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "dl",
    "dt",
    "dd",
    "blockquote",
    "pre",
    "figure",
    "figcaption",
    "table",
    "tr",
    "hr",
    "address",
    "details",
    "summary",
];

pub fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Ext A
        | 0x4E00..=0x9FFF // CJK Unified
        | 0xF900..=0xFAFF // CJK Compatibility
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0x20000..=0x2FA1F)
}

/// Collapse every run of Unicode whitespace (including NBSP) into one ASCII space.
pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{00A0}' {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// Remove single spaces sitting between two CJK characters.
pub fn remove_cjk_gaps(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == ' ' && i > 0 && i + 1 < chars.len() && is_cjk(chars[i - 1]) && is_cjk(chars[i + 1]) {
            continue;
        }
        out.push(c);
    }
    out
}

/// STRING normalization of an arbitrary string (used for values that do not come from the DOM).
pub fn normalize_string(s: &str) -> String {
    let collapsed = collapse_whitespace(&s.nfc().collect::<String>());
    remove_cjk_gaps(collapsed.trim())
}

/// TEXT normalization of an arbitrary multi-line string.
pub fn normalize_text(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    let mut lines: Vec<String> = Vec::new();
    for line in nfc.split('\n') {
        let cells: Vec<String> = line
            .split('\t')
            .map(|c| remove_cjk_gaps(collapse_whitespace(c).trim()))
            .collect();
        lines.push(cells.join("\t").trim_end().to_string());
    }
    collapse_blank_lines(&lines.join("\n"))
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0usize;
    for c in s.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out.trim_matches(|c| c == '\n' || c == ' ').to_string()
}

pub(crate) fn is_hidden(el: &ElementRef<'_>) -> bool {
    let e = el.value();
    if e.attr("hidden").is_some() {
        return true;
    }
    if e.attr("aria-hidden")
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    {
        return true;
    }
    if let Some(style) = e.attr("style") {
        let s: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        if s.contains("display:none") || s.contains("visibility:hidden") {
            return true;
        }
    }
    false
}

fn skip_descendant(el: &ElementRef<'_>) -> bool {
    let name = el.value().name();
    if SKIP_TAGS.contains(&name) || is_hidden(el) {
        return true;
    }
    // <title> inside inline SVG is a tooltip, not content.
    name == "title"
        && el
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|a| a.value().name() == "svg")
}

/// §8.1 STRING: single line.
pub fn string_text(el: ElementRef<'_>, budget: &Budget) -> Result<String, BudgetExceeded> {
    string_text_excluding(el, budget, &HashSet::new())
}

/// STRING with the subtrees rooted at `skip` removed.
pub fn string_text_excluding(
    el: ElementRef<'_>,
    budget: &Budget,
    skip: &HashSet<NodeId>,
) -> Result<String, BudgetExceeded> {
    let mut raw = String::new();
    collect_inline(*el, &mut raw, budget, true, skip)?;
    Ok(normalize_string(&raw))
}

fn collect_inline(
    node: NodeRef<'_, Node>,
    out: &mut String,
    budget: &Budget,
    is_root: bool,
    skip: &HashSet<NodeId>,
) -> Result<(), BudgetExceeded> {
    budget.spend(1)?;
    match node.value() {
        Node::Text(t) => out.push_str(t),
        Node::Element(_) => {
            let el = ElementRef::wrap(node).expect("element node");
            if !is_root && (skip_descendant(&el) || skip.contains(&node.id())) {
                return Ok(());
            }
            for child in node.children() {
                collect_inline(child, out, budget, false, skip)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Text of the direct child text nodes only, STRING-normalized.
pub fn own_text(el: ElementRef<'_>) -> String {
    let mut raw = String::new();
    for child in el.children() {
        if let Node::Text(t) = child.value() {
            raw.push_str(t);
        }
    }
    normalize_string(&raw)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Break {
    None,
    Tab,
    Line,
    Para,
}

struct BlockWriter {
    out: String,
    pending: Break,
}

impl BlockWriter {
    fn request(&mut self, b: Break) {
        if b > self.pending {
            self.pending = b;
        }
    }

    fn flush_break(&mut self) {
        let at_start = self.out.is_empty();
        match self.pending {
            Break::None => {}
            _ if at_start => {}
            Break::Tab => {
                if !self.out.ends_with('\n') {
                    self.trim_trailing_space();
                    self.out.push('\t');
                }
            }
            Break::Line => {
                self.trim_trailing_space();
                self.out.push('\n');
            }
            Break::Para => {
                self.trim_trailing_space();
                while self.out.ends_with('\n') {
                    self.out.pop();
                }
                self.out.push_str("\n\n");
            }
        }
        self.pending = Break::None;
    }

    fn trim_trailing_space(&mut self) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
    }

    fn text(&mut self, t: &str) {
        let collapsed = collapse_whitespace(t);
        if collapsed.trim().is_empty() {
            // Pure whitespace only matters between inline content on the same line.
            if self.pending == Break::None && !self.out.is_empty() && !self.out.ends_with([' ', '\n', '\t']) {
                self.out.push(' ');
            }
            return;
        }
        self.flush_break();
        let mut s = collapsed.as_str();
        if self.out.is_empty() || self.out.ends_with([' ', '\n', '\t']) {
            s = s.trim_start();
        }
        self.out.push_str(s);
    }

    fn raw(&mut self, t: &str) {
        self.flush_break();
        self.out.push_str(t);
    }
}

/// §8.2 TEXT: multi-line, block-aware.
pub fn block_text(el: ElementRef<'_>, budget: &Budget) -> Result<String, BudgetExceeded> {
    block_text_excluding(el, budget, &HashSet::new())
}

/// TEXT with the subtrees rooted at `skip` removed.
pub fn block_text_excluding(
    el: ElementRef<'_>,
    budget: &Budget,
    skip: &HashSet<NodeId>,
) -> Result<String, BudgetExceeded> {
    let mut w = BlockWriter {
        out: String::new(),
        pending: Break::None,
    };
    walk_block(*el, &mut w, budget, false, true, skip)?;
    Ok(finish_block(&w.out))
}

/// Final pass for DOM block text: whitespace was already collapsed outside `pre`, so only NFC,
/// CJK gaps, trailing spaces per line and blank-line runs are normalized (pre indentation survives).
fn finish_block(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    let lines: Vec<String> = nfc
        .split('\n')
        .map(|l| remove_cjk_gaps(l.trim_end_matches(' ')))
        .collect();
    collapse_blank_lines(&lines.join("\n"))
}

fn walk_block(
    node: NodeRef<'_, Node>,
    w: &mut BlockWriter,
    budget: &Budget,
    in_pre: bool,
    is_root: bool,
    skip: &HashSet<NodeId>,
) -> Result<(), BudgetExceeded> {
    budget.spend(1)?;
    match node.value() {
        Node::Text(t) => {
            if in_pre {
                w.raw(t);
            } else {
                w.text(t);
            }
        }
        Node::Element(e) => {
            let el = ElementRef::wrap(node).expect("element node");
            if !is_root && (skip_descendant(&el) || skip.contains(&node.id())) {
                return Ok(());
            }
            let name = e.name();
            if name == "br" {
                w.request(Break::Line);
                return Ok(());
            }
            let block = BLOCK_TAGS.contains(&name);
            let cell = name == "td" || name == "th";
            if block {
                w.request(Break::Para);
            }
            if cell {
                w.request(Break::Tab);
            }
            let pre = in_pre || name == "pre";
            for child in node.children() {
                walk_block(child, w, budget, pre, false, skip)?;
            }
            if block {
                w.request(Break::Para);
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::Html;

    fn first<'a>(doc: &'a Html, sel: &str) -> ElementRef<'a> {
        doc.select(&scraper::Selector::parse(sel).unwrap()).next().unwrap()
    }

    #[test]
    fn string_text_collapses_and_skips_hidden_and_scripts() {
        // Arrange
        let doc = Html::parse_document(
            "<div id=x>  Hello\u{00A0}\n <b>world</b><script>bad()</script><span hidden>no</span><span style='display: none'>no</span>!</div>",
        );
        let budget = Budget::new(10_000);

        // Act
        let s = string_text(first(&doc, "#x"), &budget).unwrap();

        // Assert
        assert_eq!(s, "Hello world!");
    }

    #[test]
    fn string_text_does_not_split_cjk() {
        // Arrange
        let doc = Html::parse_document("<p id=x>中文\n  文本 and English</p>");

        // Act
        let s = string_text(first(&doc, "#x"), &Budget::new(1000)).unwrap();

        // Assert
        assert_eq!(s, "中文文本 and English");
    }

    #[test]
    fn block_text_separates_paragraphs_lines_and_cells() {
        // Arrange
        let doc = Html::parse_document(
            "<article id=x><h1>Title</h1><p>One <em>two</em></p><p>Three<br>Four</p>\
             <table><tr><td>a</td><td>b</td></tr><tr><td>c</td><td>d</td></tr></table>\
             <pre>  keep\n   this</pre></article>",
        );

        // Act
        let s = block_text(first(&doc, "#x"), &Budget::new(10_000)).unwrap();

        // Assert
        assert_eq!(s, "Title\n\nOne two\n\nThree\nFour\n\na\tb\n\nc\td\n\n  keep\n   this");
    }

    #[test]
    fn budget_exhaustion_is_reported() {
        // Arrange
        let doc = Html::parse_document("<div id=x><p>a</p><p>b</p><p>c</p></div>");
        let budget = Budget::new(2);

        // Act
        let r = block_text(first(&doc, "#x"), &budget);

        // Assert
        assert!(r.is_err());
    }
}
