//! Main-content detection for the `readability` source (decisions.md U12, as amended).
//!
//! A deterministic, readability-style scorer modelled on Mozilla Readability: paragraphs vote for up to
//! five ancestors, class/id and tag weights adjust, link density penalizes, the best container wins
//! (with a parent climb and sibling merge). The container is then cleaned of boilerplate subtrees.
//! Output HTML is *not* the final sanitized value: HTML-kind fields still go through ammonia.
//!
//! Text and link lengths are computed once per document in a single post-order pass, so link density
//! is O(1) per element (a 1 MB page would otherwise re-walk large subtrees for every candidate).

use std::collections::HashMap;

use ego_tree::{NodeId, NodeRef};
use rex_dom::text::{block_text, string_text};
use rex_dom::{Budget, BudgetExceeded, Document};
use scraper::{ElementRef, Html, Node, Selector};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Readable {
    pub title: String,
    pub byline: String,
    pub excerpt: String,
    pub content_html: String,
    pub content_text: String,
}

/// Subtrees that are never content.
const DROP_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "svg", "canvas", "object", "embed", "form", "input", "button",
    "select", "textarea", "nav", "header", "footer", "aside",
];
/// Subtrees whose text never counts (mirrors rex-dom's text walkers).
const NO_TEXT_TAGS: &[&str] = &["script", "style", "noscript", "template"];
const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
];
const PARAGRAPH_TAGS: &[&str] = &["p", "pre", "blockquote", "td"];
const POSITIVE: &[&str] = &[
    "article", "body", "content", "entry", "hentry", "main", "page", "post", "text", "blog", "story",
];
const NEGATIVE: &[&str] = &[
    "metadata",
    "navbox",
    "editsection",
    "comment",
    "contact",
    "foot",
    "masthead",
    "outbrain",
    "promo",
    "related",
    "share",
    "shoutbox",
    "sidebar",
    "skyscraper",
    "sponsor",
    "shopping",
    "widget",
    "advert",
    "banner",
    "nav",
    "menu",
    "subscribe",
    "newsletter",
    "cookie",
    "popup",
    "modal",
    "recommend",
    "read-next",
    "readnext",
];
const MIN_PARAGRAPH_CHARS: usize = 25;
const EXCERPT_MAX_CHARS: usize = 300;

/// Final (link-density-adjusted) scores of every candidate container.
type Scores = HashMap<NodeId, f64>;

/// Collapsed text length and the part of it inside links, per element.
#[derive(Clone, Copy, Default)]
struct Stat {
    text: usize,
    link: usize,
}

struct Scorer<'b> {
    budget: &'b Budget,
    stats: HashMap<NodeId, Stat>,
}

pub fn extract(doc: &Document, budget: &Budget) -> Result<Readable, BudgetExceeded> {
    let scorer = Scorer {
        budget,
        stats: text_stats(doc.root(), budget)?,
    };
    let (top, scores) = scorer.best_container(doc)?;
    let content_html = match top {
        Some(el) => scorer.content_with_siblings(el, &scores)?,
        None => String::new(),
    };
    let fragment = Html::parse_fragment(&content_html);
    let content_text = block_text(fragment.root_element(), budget)?;
    let excerpt = excerpt_of(&fragment, budget)?;
    Ok(Readable {
        title: title_of(doc, budget)?,
        byline: byline_of(doc, budget)?,
        excerpt,
        content_html,
        content_text,
    })
}

/// The element the scorer picks as the main content container (before siblings are merged). Recipe
/// drafting uses it to show the model where the article body lives.
pub fn main_container<'a>(doc: &'a Document, budget: &Budget) -> Result<Option<ElementRef<'a>>, BudgetExceeded> {
    let scorer = Scorer {
        budget,
        stats: text_stats(doc.root(), budget)?,
    };
    Ok(scorer.best_container(doc)?.0)
}

/// Length of `t` with every whitespace run counted as one character.
fn collapsed_len(t: &str) -> usize {
    let mut n = 0;
    let mut in_ws = false;
    for c in t.chars() {
        if c.is_whitespace() {
            if !in_ws {
                n += 1;
            }
            in_ws = true;
        } else {
            n += 1;
            in_ws = false;
        }
    }
    n
}

/// One post-order pass over the document computing `Stat` for every element.
fn text_stats(root: ElementRef<'_>, budget: &Budget) -> Result<HashMap<NodeId, Stat>, BudgetExceeded> {
    let mut stats: HashMap<NodeId, Stat> = HashMap::new();
    // (node, children pushed?)
    let mut stack: Vec<(NodeRef<'_, Node>, bool)> = vec![(*root, false)];
    while let Some((node, expanded)) = stack.pop() {
        budget.spend(1)?;
        let Some(el) = ElementRef::wrap(node) else { continue };
        if NO_TEXT_TAGS.contains(&el.value().name()) || is_hidden(&el) {
            stats.insert(node.id(), Stat::default());
            continue;
        }
        if !expanded {
            stack.push((node, true));
            for child in node.children() {
                if matches!(child.value(), Node::Element(_)) {
                    stack.push((child, false));
                }
            }
            continue;
        }
        let mut s = Stat::default();
        for child in node.children() {
            match child.value() {
                Node::Text(t) => s.text += collapsed_len(t),
                Node::Element(_) => {
                    let c = stats.get(&child.id()).copied().unwrap_or_default();
                    s.text += c.text;
                    s.link += c.link;
                }
                _ => {}
            }
        }
        if el.value().name() == "a" {
            s.link = s.text;
        }
        stats.insert(node.id(), s);
    }
    Ok(stats)
}

fn class_weight(el: &ElementRef<'_>) -> f64 {
    let mut s = String::new();
    for a in ["class", "id"] {
        if let Some(v) = el.value().attr(a) {
            s.push(' ');
            s.push_str(&v.to_ascii_lowercase());
        }
    }
    if s.is_empty() {
        return 0.0;
    }
    let tokens: Vec<&str> = s
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut w = 0.0;
    if NEGATIVE.iter().any(|n| s.contains(n)) || tokens.iter().any(|t| *t == "ad" || *t == "ads") {
        w -= 25.0;
    }
    if POSITIVE.iter().any(|p| s.contains(p)) {
        w += 25.0;
    }
    w
}

fn tag_weight(name: &str) -> f64 {
    match name {
        "article" => 10.0,
        "div" | "section" | "main" => 5.0,
        "pre" | "td" | "blockquote" => 3.0,
        "address" | "ol" | "ul" | "dl" | "dd" | "dt" | "li" | "form" => -3.0,
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th" => -5.0,
        _ => 0.0,
    }
}

/// A `<form>` holding article paragraphs is a page wrapper (ASP.NET WebForms puts the whole body in one), not
/// a form: only forms with fewer paragraphs than this are boilerplate.
const FORM_WRAPPER_PARAGRAPHS: usize = 3;

fn is_dropped_tag(el: &ElementRef<'_>) -> bool {
    let name = el.value().name();
    if name == "form" {
        let paragraphs = el
            .descendants()
            .filter_map(ElementRef::wrap)
            .filter(|d| d.value().name() == "p")
            .take(FORM_WRAPPER_PARAGRAPHS)
            .count();
        return paragraphs < FORM_WRAPPER_PARAGRAPHS;
    }
    DROP_TAGS.contains(&name)
}

fn in_dropped_subtree(el: &ElementRef<'_>) -> bool {
    el.ancestors().filter_map(ElementRef::wrap).any(|a| is_dropped_tag(&a))
}

impl Scorer<'_> {
    fn stat(&self, el: ElementRef<'_>) -> Stat {
        self.stats.get(&el.id()).copied().unwrap_or_default()
    }

    fn text_len(&self, el: ElementRef<'_>) -> usize {
        self.stat(el).text
    }

    fn link_density(&self, el: ElementRef<'_>) -> f64 {
        let s = self.stat(el);
        if s.text == 0 {
            return 1.0;
        }
        (s.link as f64 / s.text as f64).min(1.0)
    }

    fn best_container<'a>(&self, doc: &'a Document) -> Result<(Option<ElementRef<'a>>, Scores), BudgetExceeded> {
        let body = Selector::parse("body").expect("static selector");
        let scope = doc.root().select(&body).next().unwrap_or_else(|| doc.root());
        let para_sel = Selector::parse(&PARAGRAPH_TAGS.join(",")).expect("static selector");

        let mut scores: HashMap<NodeId, f64> = HashMap::new();
        let mut order: Vec<NodeId> = Vec::new();
        let mut add = |el: ElementRef<'_>, amount: f64, order: &mut Vec<NodeId>| {
            let entry = scores.entry(el.id()).or_insert_with(|| {
                order.push(el.id());
                tag_weight(el.value().name()) + class_weight(&el)
            });
            *entry += amount;
        };

        for p in scope.select(&para_sel) {
            self.budget.spend(1)?;
            if in_dropped_subtree(&p) || self.text_len(p) < MIN_PARAGRAPH_CHARS {
                continue;
            }
            let text = string_text(p, self.budget)?;
            let len = text.chars().count();
            if len < MIN_PARAGRAPH_CHARS {
                continue;
            }
            let commas = text.matches([',', '，', '、']).count() as f64;
            let score = 1.0 + commas + (len as f64 / 100.0).min(3.0);
            // Mozilla Readability: up to 5 ancestors, divided by 1, 2, then level * 3.
            for (level, ancestor) in p.ancestors().filter_map(ElementRef::wrap).take(5).enumerate() {
                if matches!(ancestor.value().name(), "body" | "html") {
                    break;
                }
                let divider = match level {
                    0 => 1.0,
                    1 => 2.0,
                    n => n as f64 * 3.0,
                };
                add(ancestor, score / divider, &mut order);
            }
        }

        let mut best: Option<(f64, NodeId)> = None;
        let mut finals: Scores = HashMap::new();
        for id in &order {
            let node = doc.root().tree().get(*id).expect("scored node exists");
            let el = ElementRef::wrap(node).expect("scored nodes are elements");
            let final_score = scores[id] * (1.0 - self.link_density(el));
            finals.insert(*id, final_score);
            if best.is_none_or(|(s, _)| final_score > s) {
                best = Some((final_score, *id));
            }
        }
        if let Some((score, id)) = best {
            let top = ElementRef::wrap(doc.root().tree().get(id).expect("best node")).expect("element");
            let top = shared_ancestor_of_alternatives(top, score, &order, &finals);
            let top_score = finals.get(&top.id()).copied().unwrap_or(score);
            let top = climb(top, top_score, &finals);
            return Ok((Some(only_child_up(top)), finals));
        }
        for fallback in ["article", "main", "[role=main]", "body"] {
            let s = Selector::parse(fallback).expect("static selector");
            if let Some(el) = doc.root().select(&s).next() {
                return Ok((Some(el), finals));
            }
        }
        Ok((None, finals))
    }

    /// The best container plus siblings that also look like content (Mozilla Readability's sibling
    /// step): a sibling joins if its own score (plus 20% of the best score when it shares the best
    /// candidate's class) is >= max(10, 20% of the best score), or it is a paragraph with enough text
    /// and few links. Handles pages that wrap each section in its own element.
    fn content_with_siblings(&self, top: ElementRef<'_>, scores: &Scores) -> Result<String, BudgetExceeded> {
        let Some(parent) = top.parent().and_then(ElementRef::wrap) else {
            return self.clean_inner_html(top);
        };
        let top_score = scores.get(&top.id()).copied().unwrap_or(0.0);
        let threshold = (top_score * 0.2).max(10.0);
        let mut out = String::new();
        for sib in parent.children().filter_map(ElementRef::wrap) {
            self.budget.spend(1)?;
            if sib.id() == top.id() {
                out.push_str(&self.clean_inner_html(sib)?);
                continue;
            }
            if self.is_boilerplate(sib) {
                continue;
            }
            // Mozilla: siblings sharing the top candidate's (non-empty) class get a 20% bonus.
            let top_class = top.value().attr("class").unwrap_or_default();
            let bonus = if !top_class.is_empty() && sib.value().attr("class") == Some(top_class) {
                top_score * 0.2
            } else {
                0.0
            };
            let include = match scores.get(&sib.id()) {
                Some(s) if *s + bonus >= threshold => true,
                _ if sib.value().name() == "p" => {
                    let len = self.text_len(sib);
                    let density = self.link_density(sib);
                    (len > 80 && density < 0.25) || (len >= MIN_PARAGRAPH_CHARS && density == 0.0)
                }
                _ => false,
            };
            if include {
                self.serialize(*sib, &mut out)?;
            }
        }
        Ok(out)
    }

    /// Boilerplate inside the chosen container: dropped tags, hidden elements, the headline and byline,
    /// and negative class/id with little text or many links.
    fn is_boilerplate(&self, el: ElementRef<'_>) -> bool {
        if is_dropped_tag(&el) || is_hidden(&el) {
            return true;
        }
        // The headline and byline are separate fields (title, byline), not body content.
        if el.value().name() == "h1" {
            return true;
        }
        if is_byline(&el) && self.text_len(el) <= 100 {
            return true;
        }
        if class_weight(&el) < 0.0 {
            return self.text_len(el) < 200 || self.link_density(el) > 0.33;
        }
        false
    }

    fn clean_inner_html(&self, el: ElementRef<'_>) -> Result<String, BudgetExceeded> {
        let mut out = String::new();
        for child in el.children() {
            self.serialize(child, &mut out)?;
        }
        Ok(out)
    }

    fn serialize(&self, node: NodeRef<'_, Node>, out: &mut String) -> Result<(), BudgetExceeded> {
        self.budget.spend(1)?;
        match node.value() {
            Node::Text(t) => escape_text(t, out),
            Node::Element(e) => {
                let el = ElementRef::wrap(node).expect("element");
                if self.is_boilerplate(el) {
                    return Ok(());
                }
                out.push('<');
                out.push_str(e.name());
                for (k, v) in e.attrs() {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    escape_attr(v, out);
                    out.push('"');
                }
                out.push('>');
                if VOID_TAGS.contains(&e.name()) {
                    return Ok(());
                }
                for child in node.children() {
                    self.serialize(child, out)?;
                }
                out.push_str("</");
                out.push_str(e.name());
                out.push('>');
            }
            _ => {}
        }
        Ok(())
    }
}

/// Mozilla Readability's "alternative candidates" step: when at least three of the top five candidates
/// score >= 75% of the best and share an ancestor with it, that ancestor becomes the top candidate
/// (pages that split the body into many similar blocks, e.g. one component per paragraph group).
fn shared_ancestor_of_alternatives<'a>(
    top: ElementRef<'a>,
    top_score: f64,
    order: &[NodeId],
    scores: &Scores,
) -> ElementRef<'a> {
    const MIN_ALTERNATIVES: usize = 3;
    let mut ranked: Vec<(f64, usize, NodeId)> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (scores.get(id).copied().unwrap_or(0.0), i, *id))
        .collect();
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let alternatives: Vec<NodeId> = ranked
        .iter()
        .take(5)
        .filter(|(s, _, id)| *id != top.id() && top_score > 0.0 && s / top_score >= 0.75)
        .map(|(_, _, id)| *id)
        .collect();
    if alternatives.len() < MIN_ALTERNATIVES {
        return top;
    }
    let tree = top.tree();
    let mut cur = top;
    while let Some(parent) = cur.parent().and_then(ElementRef::wrap) {
        if matches!(parent.value().name(), "body" | "html") {
            break;
        }
        let contained = alternatives
            .iter()
            .filter(|id| {
                tree.get(**id)
                    .is_some_and(|n| n.ancestors().any(|a| a.id() == parent.id()))
            })
            .count();
        if contained >= MIN_ALTERNATIVES {
            return parent;
        }
        cur = parent;
    }
    top
}

/// Mozilla Readability: an only child is replaced by its parent (so the sibling step sees real siblings).
fn only_child_up(top: ElementRef<'_>) -> ElementRef<'_> {
    let mut cur = top;
    while let Some(parent) = cur.parent().and_then(ElementRef::wrap) {
        if matches!(parent.value().name(), "body" | "html")
            || parent.children().filter_map(ElementRef::wrap).count() != 1
        {
            break;
        }
        cur = parent;
    }
    cur
}

/// Mozilla Readability's parent climb: walk up while scored ancestors keep at least a third of the
/// top score; switch to the first ancestor that scores higher than the previous scored ancestor (e.g.
/// the wrapper of many per-section elements). Unscored ancestors are skipped.
fn climb<'a>(top: ElementRef<'a>, top_score: f64, scores: &Scores) -> ElementRef<'a> {
    let threshold = top_score / 3.0;
    let mut last = top_score;
    let mut cur = top;
    while let Some(parent) = cur.parent().and_then(ElementRef::wrap) {
        if matches!(parent.value().name(), "body" | "html") {
            break;
        }
        cur = parent;
        let Some(&ps) = scores.get(&parent.id()) else { continue };
        if ps < threshold {
            break;
        }
        if ps > last {
            return parent;
        }
        last = ps;
    }
    top
}

fn is_byline(el: &ElementRef<'_>) -> bool {
    let e = el.value();
    let attr_has = |a: &str, needle: &str| e.attr(a).is_some_and(|v| v.to_ascii_lowercase().contains(needle));
    attr_has("class", "byline")
        || attr_has("class", "author")
        || attr_has("rel", "author")
        || attr_has("itemprop", "author")
}

fn is_hidden(el: &ElementRef<'_>) -> bool {
    let e = el.value();
    e.attr("hidden").is_some()
        || e.attr("aria-hidden")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
        || e.attr("style").is_some_and(|s| {
            let s: String = s
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
                .to_ascii_lowercase();
            s.contains("display:none") || s.contains("visibility:hidden")
        })
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

fn title_of(doc: &Document, budget: &Budget) -> Result<String, BudgetExceeded> {
    let title_sel = Selector::parse("head title, title").expect("static selector");
    let raw = match doc.root().select(&title_sel).next() {
        Some(t) => string_text(t, budget)?,
        None => String::new(),
    };
    if !raw.is_empty() {
        for sep in [" | ", " - ", " — ", " – ", " :: ", " · "] {
            if let Some((head, _)) = raw.split_once(sep) {
                let words = head.split_whitespace().count();
                let cjk = head.chars().filter(|c| rex_dom::text::is_cjk(*c)).count();
                if words >= 3 || cjk >= 6 {
                    return Ok(head.trim().to_string());
                }
            }
        }
        return Ok(raw);
    }
    let h1 = Selector::parse("h1").expect("static selector");
    match doc.root().select(&h1).next() {
        Some(h) => string_text(h, budget),
        None => Ok(String::new()),
    }
}

fn byline_of(doc: &Document, budget: &Budget) -> Result<String, BudgetExceeded> {
    for sel in [
        "[rel~=author]",
        "[itemprop~=author]",
        ".byline",
        "[class*=byline]",
        ".author",
        "[class*=author]",
    ] {
        let s = Selector::parse(sel).expect("static selector");
        for el in doc.root().select(&s) {
            if el.value().name() == "meta" || el.value().name() == "link" {
                continue;
            }
            let t = string_text(el, budget)?;
            let t = strip_by_prefix(&t);
            let n = t.chars().count();
            if n > 0 && n <= 100 {
                return Ok(t);
            }
        }
    }
    Ok(String::new())
}

fn strip_by_prefix(s: &str) -> String {
    let t = s.trim();
    for p in ["By ", "by ", "BY ", "作者：", "作者:", "文/"] {
        if let Some(rest) = t.strip_prefix(p) {
            return rest.trim().to_string();
        }
    }
    t.to_string()
}

fn excerpt_of(fragment: &Html, budget: &Budget) -> Result<String, BudgetExceeded> {
    let p = Selector::parse("p").expect("static selector");
    for el in fragment.select(&p) {
        let t = string_text(el, budget)?;
        if t.chars().count() >= 40 {
            return Ok(t.chars().take(EXCERPT_MAX_CHARS).collect());
        }
    }
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_wrapping_the_whole_page_keeps_its_article() {
        // Arrange: ASP.NET WebForms wraps the body in one <form>
        let paras: String = (0..5)
            .map(|i| format!("<p>Paragraph {i} of the story, long enough to count as real content for scoring purposes here.</p>"))
            .collect();
        let html = format!(
            r#"<html><body><form id="aspnetForm"><input type="hidden" name="__VIEWSTATE"><div class="article">{paras}</div></form></body></html>"#
        );
        let doc = Document::parse(&html, "https://e.com/a", &rex_dom::Limits::default()).unwrap();

        // Act
        let r = extract(&doc, &Budget::new(1_000_000)).unwrap();

        // Assert
        assert!(r.content_text.contains("Paragraph 4"), "{:?}", r.content_text);
    }
    use rex_dom::Limits;

    const PAGE: &str = r#"<html><head><title>Fed Holds Rates Steady as Inflation Cools | The Daily Example</title></head>
<body>
<nav class="menu"><a href="/">Home</a> <a href="/markets">Markets</a></nav>
<div class="layout">
  <div class="sidebar"><p>Most popular: a very long list of links to other stories that are not content.</p></div>
  <article class="story">
    <h1>Fed Holds Rates Steady as Inflation Cools</h1>
    <div class="byline">By Jane Doe</div>
    <section class="article-body">
      <p>Federal Reserve officials held interest rates steady on Wednesday, signaling patience as price growth slowed.</p>
      <div class="ad">Advertisement</div>
      <p>Markets had largely priced in the decision, and Treasury yields moved little after the announcement, analysts said.</p>
      <div class="read-next"><a href="/x">Read next: Markets rally after the decision</a></div>
    </section>
  </article>
</div>
<footer><p>Copyright notice and a long list of legal text that is not content at all.</p></footer>
</body></html>"#;

    #[test]
    fn picks_article_body_and_drops_boilerplate() {
        // Arrange
        let doc = Document::parse(PAGE, "https://news.example/a/1", &Limits::default()).unwrap();
        let budget = Budget::new(1_000_000);

        // Act
        let r = extract(&doc, &budget).unwrap();

        // Assert
        assert!(r.content_text.starts_with("Federal Reserve officials held"));
        assert!(r.content_text.contains("Treasury yields moved little"));
        assert!(!r.content_text.contains("Advertisement"));
        assert!(!r.content_text.contains("Read next"));
        assert!(!r.content_text.contains("Most popular"));
        assert!(!r.content_text.contains("Copyright"));
    }

    #[test]
    fn sibling_sections_are_merged() {
        // Arrange
        let html = r#"<html><body><div id="content">
          <section><h2>History</h2><p>The first section has a long paragraph about history, with commas, clauses, and detail.</p><p>Another paragraph in the first section, long enough to count as content for scoring.</p></section>
          <section><h2>Methods</h2><p>The second section is longer: it has three paragraphs, many commas, and lots of words, which makes it the best.</p><p>More method detail goes here, again long enough to be counted as a real paragraph of content.</p><p>And a third paragraph, so that this section clearly wins the scoring over its siblings.</p></section>
          <section class="related"><a href="/a">Related link one</a> <a href="/b">Related link two</a></section>
        </div></body></html>"#;
        let doc = Document::parse(html, "https://wiki.example/x", &Limits::default()).unwrap();

        // Act
        let r = extract(&doc, &Budget::new(1_000_000)).unwrap();

        // Assert
        assert!(r.content_text.contains("The first section has a long paragraph"));
        assert!(r.content_text.contains("The second section is longer"));
        assert!(!r.content_text.contains("Related link"));
    }

    #[test]
    fn link_density_comes_from_one_pass_stats() {
        // Arrange
        let doc = Document::parse(
            r#"<div id=d>abcde <a href=/x>fghij</a><script>ignored text</script></div>"#,
            "https://e.com/",
            &Limits::default(),
        )
        .unwrap();
        let budget = Budget::new(10_000);
        let scorer = Scorer {
            budget: &budget,
            stats: text_stats(doc.root(), &budget).unwrap(),
        };
        let d = doc.root().select(&Selector::parse("#d").unwrap()).next().unwrap();

        // Act
        let density = scorer.link_density(d);

        // Assert
        assert_eq!(scorer.text_len(d), 11);
        assert!((density - 5.0 / 11.0).abs() < 1e-9);
    }

    #[test]
    fn title_byline_excerpt() {
        // Arrange
        let doc = Document::parse(PAGE, "https://news.example/a/1", &Limits::default()).unwrap();
        let budget = Budget::new(1_000_000);

        // Act
        let r = extract(&doc, &budget).unwrap();

        // Assert
        assert_eq!(r.title, "Fed Holds Rates Steady as Inflation Cools");
        assert_eq!(r.byline, "Jane Doe");
        assert!(r
            .excerpt
            .starts_with("Federal Reserve officials held interest rates steady"));
    }
}
