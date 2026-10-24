//! Read-only document model for rex (engine spec §4, §8, §10.3).
//!
//! A [`Document`] is parsed once and shared by every recipe run on the page. Indexes (meta, JSON-LD,
//! microdata) are built lazily on first use.

use std::cell::{Cell, OnceCell};

use scraper::{ElementRef, Html, Selector};
use url::Url;

pub mod jsonld;
pub mod microdata;
pub mod text;

pub use scraper::ElementRef as Element;

/// Hard limits (engine spec §10.3).
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_html_bytes: usize,
    pub max_nodes: usize,
    pub fuel: u64,
    pub max_items: u32,
    pub max_depth: u32,
    pub max_output_bytes: usize,
    pub max_selector_len: usize,
    pub max_regex_bytes: usize,
    pub max_rules: usize,
    pub max_extends_depth: usize,
    /// Element nesting kept from a page; deeper subtrees are dropped at parse so that no tree walk (several
    /// recurse) can overflow the stack, which is small under wasm.
    pub max_dom_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_html_bytes: 10 * 1024 * 1024,
            max_nodes: 500_000,
            fuel: 50_000_000,
            max_items: 1000,
            max_depth: 8,
            max_output_bytes: 4 * 1024 * 1024,
            max_selector_len: 512,
            max_regex_bytes: 1024 * 1024,
            max_rules: 256,
            max_extends_depth: 3,
            max_dom_depth: MAX_DOM_DEPTH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("fuel budget exceeded")]
pub struct BudgetExceeded;

/// Deterministic work counter: every node visit, selector test, regex call and transform step
/// spends fuel. Exhaustion is an error, never a hang.
#[derive(Debug)]
pub struct Budget {
    used: Cell<u64>,
    limit: u64,
}

impl Budget {
    pub fn new(limit: u64) -> Self {
        Self {
            used: Cell::new(0),
            limit,
        }
    }

    pub fn spend(&self, n: u64) -> Result<(), BudgetExceeded> {
        let next = self.used.get().saturating_add(n);
        self.used.set(next);
        if next > self.limit {
            Err(BudgetExceeded)
        } else {
            Ok(())
        }
    }

    pub fn used(&self) -> u64 {
        self.used.get()
    }

    pub fn exhausted(&self) -> bool {
        self.used.get() > self.limit
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DomError {
    #[error("page URL is invalid: {0}")]
    BadUrl(#[from] url::ParseError),
    #[error("HTML is {size} bytes, over the {limit}-byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("document has {nodes} nodes, over the {limit}-node limit")]
    TooManyNodes { nodes: usize, limit: usize },
}

/// A compiled CSS selector with rex scoping rules (U7): an empty selector is the scope element
/// itself; a selector starting with a combinator (or containing `:scope`) is matched relative to the
/// scope element, so sibling combinators work.
#[derive(Debug, Clone)]
pub enum Sel {
    SelfElement,
    Descendant(Selector),
    Relative(Selector),
}

impl Sel {
    pub fn parse(src: &str, max_len: usize) -> Result<Sel, String> {
        let s = src.trim();
        if s.len() > max_len {
            return Err(format!("selector is {} chars, over the {max_len}-char limit", s.len()));
        }
        if s.is_empty() {
            return Ok(Sel::SelfElement);
        }
        let relative = s.starts_with(['+', '~', '>']) || s.contains(":scope");
        let text = if s.starts_with(['+', '~', '>']) {
            format!(":scope {s}")
        } else {
            s.to_string()
        };
        let selector = Selector::parse(&text).map_err(|e| format!("invalid CSS selector `{s}`: {e:?}"))?;
        Ok(if relative {
            Sel::Relative(selector)
        } else {
            Sel::Descendant(selector)
        })
    }
}

/// Default `Limits::max_dom_depth`. Real pages nest well under 100 levels.
pub const MAX_DOM_DEPTH: usize = 256;

/// Detaches every subtree that starts deeper than `max_depth` (iteratively, so the walk itself cannot
/// overflow); returns how many subtrees were dropped.
pub fn prune_deep(html: &mut Html, max_depth: usize) -> usize {
    let mut too_deep = Vec::new();
    let mut stack = vec![(html.tree.root().id(), 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = html.tree.get(id) else { continue };
        for child in node.children() {
            if depth + 1 > max_depth {
                too_deep.push(child.id());
            } else {
                stack.push((child.id(), depth + 1));
            }
        }
    }
    for id in &too_deep {
        if let Some(mut n) = html.tree.get_mut(*id) {
            n.detach();
        }
    }
    too_deep.len()
}

pub struct Document {
    html: Html,
    url: Url,
    base: Url,
    meta: OnceCell<Vec<(String, String)>>,
    jsonld: OnceCell<jsonld::JsonLdIndex>,
    microdata: OnceCell<Vec<serde_json::Value>>,
}

impl Document {
    pub fn parse(html: &str, page_url: &str, limits: &Limits) -> Result<Document, DomError> {
        if html.len() > limits.max_html_bytes {
            return Err(DomError::TooLarge {
                size: html.len(),
                limit: limits.max_html_bytes,
            });
        }
        let url = Url::parse(page_url)?;
        let mut parsed = Html::parse_document(html);
        prune_deep(&mut parsed, limits.max_dom_depth);
        let nodes = parsed.tree.nodes().count();
        if nodes > limits.max_nodes {
            return Err(DomError::TooManyNodes {
                nodes,
                limit: limits.max_nodes,
            });
        }
        let base = Selector::parse("base[href]")
            .ok()
            .and_then(|s| parsed.select(&s).next())
            .and_then(|b| b.value().attr("href"))
            .and_then(|h| url.join(h.trim()).ok())
            .unwrap_or_else(|| url.clone());
        Ok(Document {
            html: parsed,
            url,
            base,
            meta: OnceCell::new(),
            jsonld: OnceCell::new(),
            microdata: OnceCell::new(),
        })
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn root(&self) -> ElementRef<'_> {
        self.html.root_element()
    }

    /// Resolve a possibly relative URL against the document base.
    pub fn resolve_url(&self, raw: &str) -> Option<Url> {
        let t = raw.trim();
        if t.is_empty() {
            return None;
        }
        self.base.join(t).ok()
    }

    /// Elements matched by `sel` within `scope`, in document order.
    pub fn select<'a>(
        &'a self,
        scope: ElementRef<'a>,
        sel: &Sel,
        budget: &Budget,
    ) -> Result<Vec<ElementRef<'a>>, BudgetExceeded> {
        self.select_limited(scope, sel, budget, usize::MAX)
    }

    /// `select`, stopping after `limit` matches (an existence test needs one; a field never more than a few
    /// thousand).
    pub fn select_limited<'a>(
        &'a self,
        scope: ElementRef<'a>,
        sel: &Sel,
        budget: &Budget,
        limit: usize,
    ) -> Result<Vec<ElementRef<'a>>, BudgetExceeded> {
        match sel {
            Sel::SelfElement => Ok(vec![scope]),
            Sel::Descendant(s) => {
                // Walk the subtree ourselves so every node tested costs fuel (a selector that
                // matches nothing on a huge DOM must not be free).
                let mut out = Vec::new();
                for node in scope.descendants().skip(1) {
                    budget.spend(1)?;
                    if let Some(el) = ElementRef::wrap(node) {
                        if s.matches_with_scope(&el, Some(scope)) {
                            out.push(el);
                            if out.len() >= limit {
                                break;
                            }
                        }
                    }
                }
                Ok(out)
            }
            Sel::Relative(s) => {
                // Candidates live under the scope's parent (siblings and their subtrees) or the root.
                let container = scope.parent().and_then(ElementRef::wrap).unwrap_or_else(|| self.root());
                let mut out = Vec::new();
                for node in container.descendants() {
                    budget.spend(1)?;
                    if let Some(el) = ElementRef::wrap(node) {
                        if s.matches_with_scope(&el, Some(scope)) {
                            out.push(el);
                            if out.len() >= limit {
                                break;
                            }
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    /// `content` of `meta[property=key]` / `meta[name=key]` (case-insensitive key), document order.
    pub fn meta(&self, key: &str) -> Vec<String> {
        let want = key.trim().to_ascii_lowercase();
        self.meta_index()
            .iter()
            .filter(|(k, _)| *k == want)
            .map(|(_, v)| v.clone())
            .collect()
    }

    fn meta_index(&self) -> &Vec<(String, String)> {
        self.meta.get_or_init(|| {
            let sel = Selector::parse("meta[content]").expect("static selector");
            let mut out = Vec::new();
            for m in self.html.select(&sel) {
                let content = m.value().attr("content").unwrap_or_default().to_string();
                for attr in ["property", "name", "itemprop"] {
                    if let Some(k) = m.value().attr(attr) {
                        out.push((k.trim().to_ascii_lowercase(), content.clone()));
                    }
                }
            }
            out
        })
    }

    /// `href` of `link` elements whose `rel` contains `rel` (case-insensitive token), document order.
    pub fn link(&self, rel: &str) -> Vec<String> {
        let want = rel.trim().to_ascii_lowercase();
        let sel = Selector::parse("link[rel][href]").expect("static selector");
        self.html
            .select(&sel)
            .filter(|l| {
                l.value()
                    .attr("rel")
                    .is_some_and(|r| r.split_ascii_whitespace().any(|t| t.eq_ignore_ascii_case(&want)))
            })
            .filter_map(|l| l.value().attr("href").map(str::to_string))
            .collect()
    }

    pub fn jsonld(&self) -> &jsonld::JsonLdIndex {
        self.jsonld.get_or_init(|| jsonld::JsonLdIndex::build(&self.html))
    }

    /// Microdata items as JSON objects (`@type` = itemtype short name).
    pub fn microdata(&self) -> &Vec<serde_json::Value> {
        self.microdata.get_or_init(|| microdata::extract_items(self))
    }

    /// Inner HTML of `el` with the subtrees rooted at `skip` removed.
    pub fn inner_html_excluding(
        &self,
        el: ElementRef<'_>,
        skip: &std::collections::HashSet<ego_tree::NodeId>,
    ) -> String {
        let mut out = String::new();
        for child in el.children() {
            serialize_node(child, skip, &mut out);
        }
        out
    }

    /// Outer HTML of `el` with the subtrees rooted at `skip` removed.
    pub fn outer_html_excluding(
        &self,
        el: ElementRef<'_>,
        skip: &std::collections::HashSet<ego_tree::NodeId>,
    ) -> String {
        let mut out = String::new();
        serialize_node(*el, skip, &mut out);
        out
    }

    /// The `html` element's `lang` attribute.
    pub fn lang(&self) -> Option<String> {
        self.root()
            .value()
            .attr("lang")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
];

fn serialize_node(
    node: ego_tree::NodeRef<'_, scraper::Node>,
    skip: &std::collections::HashSet<ego_tree::NodeId>,
    out: &mut String,
) {
    use scraper::Node;
    match node.value() {
        Node::Text(t) => {
            for c in t.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    _ => out.push(c),
                }
            }
        }
        Node::Element(e) => {
            if skip.contains(&node.id()) {
                return;
            }
            out.push('<');
            out.push_str(e.name());
            for (k, v) in e.attrs() {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&v.replace('&', "&amp;").replace('"', "&quot;"));
                out.push('"');
            }
            out.push('>');
            if VOID_TAGS.contains(&e.name()) {
                return;
            }
            for child in node.children() {
                serialize_node(child, skip, out);
            }
            out.push_str("</");
            out.push_str(e.name());
            out.push('>');
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deeply_nested_pages_are_pruned_instead_of_overflowing_the_stack() {
        // Arrange: 50,000 nested spans around some text
        let html = format!(
            "<html><body><p>kept</p>{}deep{}</body></html>",
            "<span>".repeat(50_000),
            "</span>".repeat(50_000)
        );

        // Act
        let doc = Document::parse(&html, "https://e.com/", &Limits::default()).expect("parses");
        let text = crate::text::block_text(doc.root(), &Budget::new(10_000_000)).expect("walks");

        // Assert
        assert!(text.contains("kept"));
        assert!(!text.contains("deep"), "the over-deep subtree is dropped");
    }

    fn doc(html: &str) -> Document {
        Document::parse(html, "https://example.com/a/b?x=1", &Limits::default()).unwrap()
    }

    #[test]
    fn relative_selector_matches_following_sibling_subtree() {
        // Arrange
        let d = doc(
            "<table><tr class=athing id=1><td>t</td></tr><tr><td><span class=score>5 points</span></td></tr></table>",
        );
        let budget = Budget::new(10_000);
        let item = d
            .select(d.root(), &Sel::parse("tr.athing", 512).unwrap(), &budget)
            .unwrap()[0];

        // Act
        let hits = d
            .select(item, &Sel::parse("+ tr span.score", 512).unwrap(), &budget)
            .unwrap();

        // Assert
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].inner_html(), "5 points");
    }

    #[test]
    fn empty_selector_is_the_scope_element() {
        // Arrange
        let d = doc("<p id=a>x</p>");
        let budget = Budget::new(100);
        let p = d.select(d.root(), &Sel::parse("p", 512).unwrap(), &budget).unwrap()[0];

        // Act
        let hits = d.select(p, &Sel::parse("", 512).unwrap(), &budget).unwrap();

        // Assert
        assert_eq!(hits[0].value().attr("id"), Some("a"));
    }

    #[test]
    fn base_href_drives_url_resolution() {
        // Arrange
        let d = doc("<head><base href='/root/'></head><body></body>");

        // Act
        let u = d.resolve_url("img/x.png").unwrap();

        // Assert
        assert_eq!(u.as_str(), "https://example.com/root/img/x.png");
    }

    #[test]
    fn meta_lookup_is_case_insensitive_and_ordered() {
        // Arrange
        let d = doc("<meta property='article:tag' content='a'><meta property='ARTICLE:TAG' content='b'><meta name=author content=Jane>");

        // Act
        let tags = d.meta("article:tag");

        // Assert
        assert_eq!(tags, vec!["a", "b"]);
        assert_eq!(d.meta("Author"), vec!["Jane"]);
    }

    #[test]
    fn inner_html_excluding_drops_subtrees() {
        // Arrange
        let d = doc("<section id=s><p>keep</p><div class=ad>Advertisement</div><p>also</p></section>");
        let budget = Budget::new(1000);
        let s = d.select(d.root(), &Sel::parse("#s", 512).unwrap(), &budget).unwrap()[0];
        let skip: std::collections::HashSet<_> = s
            .select(&scraper::Selector::parse(".ad").unwrap())
            .map(|e| e.id())
            .collect();

        // Act
        let html = d.inner_html_excluding(s, &skip);
        let text = text::block_text_excluding(s, &budget, &skip).unwrap();

        // Assert
        assert_eq!(html, "<p>keep</p><p>also</p>");
        assert_eq!(text, "keep\n\nalso");
    }

    #[test]
    fn oversized_html_is_rejected() {
        // Arrange
        let limits = Limits {
            max_html_bytes: 10,
            ..Limits::default()
        };

        // Act
        let r = Document::parse("<p>0123456789</p>", "https://e.com/", &limits);

        // Assert
        assert!(matches!(r, Err(DomError::TooLarge { .. })));
    }
}
