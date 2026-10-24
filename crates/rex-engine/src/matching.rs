//! Two-stage recipe matching with a schema.org fallback (decisions.md U2).

use regex::Regex;
use rex_dom::{Budget, Document, Limits, Sel};
use rex_proto::extract as pb;
use url::Url;

use crate::transform::compile_regex;

#[derive(Clone, Debug)]
enum HostPat {
    Exact(String),
    Wildcard(String),
}

#[derive(Clone, Debug)]
enum PageCond {
    Exists(Sel),
    NotExists(Sel),
    JsonLdType(Vec<String>),
    MetaEquals(String, String),
}

#[derive(Clone, Debug)]
pub struct CompiledMatch {
    hosts: Vec<HostPat>,
    include: Vec<Regex>,
    exclude: Vec<Regex>,
    any: Vec<PageCond>,
    all: Vec<PageCond>,
    pub priority: i32,
}

/// Outcome of stage 1 for one recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlHit {
    pub exact_host: bool,
    pub include_matched: bool,
    pub priority: i32,
}

fn norm_host(h: &str) -> String {
    let h = h.trim().trim_end_matches('.').to_ascii_lowercase();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

impl CompiledMatch {
    pub fn compile(m: &pb::RecipeMatch, limits: &Limits) -> Result<CompiledMatch, String> {
        let urls = m.urls.clone().unwrap_or_default();
        let hosts = urls
            .hosts
            .iter()
            .map(|h| match h.trim().strip_prefix("*.") {
                Some(rest) => HostPat::Wildcard(norm_host(rest)),
                None => HostPat::Exact(norm_host(h)),
            })
            .collect();
        let re = |p: &String| compile_regex(p, limits.max_regex_bytes);
        let include = urls.include.iter().map(re).collect::<Result<_, _>>()?;
        let exclude = urls.exclude.iter().map(re).collect::<Result<_, _>>()?;
        let page = m.page.clone().unwrap_or_default();
        let cond = |c: &pb::PageCondition| -> Result<PageCond, String> {
            use pb::page_condition::Condition as C;
            match c.condition.as_ref().ok_or("empty page condition")? {
                C::Exists(s) => Ok(PageCond::Exists(Sel::parse(s, limits.max_selector_len)?)),
                C::NotExists(s) => Ok(PageCond::NotExists(Sel::parse(s, limits.max_selector_len)?)),
                C::JsonLdType(t) => Ok(PageCond::JsonLdType(t.types.clone())),
                C::Meta(m) => Ok(PageCond::MetaEquals(m.key.clone(), m.value.clone())),
            }
        };
        Ok(CompiledMatch {
            hosts,
            include,
            exclude,
            any: page.any.iter().map(cond).collect::<Result<_, _>>()?,
            all: page.all.iter().map(cond).collect::<Result<_, _>>()?,
            priority: m.priority,
        })
    }

    /// Stage 1: URL rules only.
    pub fn match_url(&self, url: &Url) -> Option<UrlHit> {
        let host = norm_host(url.host_str()?);
        let exact = self.hosts.iter().any(|h| matches!(h, HostPat::Exact(x) if *x == host));
        let wildcard = self
            .hosts
            .iter()
            .any(|h| matches!(h, HostPat::Wildcard(x) if host.ends_with(&format!(".{x}"))));
        if !exact && !wildcard {
            return None;
        }
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        };
        let include_matched = if self.include.is_empty() {
            false
        } else if self.include.iter().any(|r| r.is_match(&path)) {
            true
        } else {
            return None;
        };
        if self.exclude.iter().any(|r| r.is_match(&path)) {
            return None;
        }
        Some(UrlHit {
            exact_host: exact,
            include_matched,
            priority: self.priority,
        })
    }

    /// Stage 2: page conditions on the parsed document.
    pub fn match_page(&self, doc: &Document, budget: &Budget) -> bool {
        let eval = |c: &PageCond| -> bool {
            match c {
                PageCond::Exists(s) => doc
                    .select_limited(doc.root(), s, budget, 1)
                    .map(|v| !v.is_empty())
                    .unwrap_or(false),
                PageCond::NotExists(s) => doc
                    .select_limited(doc.root(), s, budget, 1)
                    .map(|v| v.is_empty())
                    .unwrap_or(false),
                PageCond::JsonLdType(types) => doc.jsonld().nodes_of_types(types).next().is_some(),
                PageCond::MetaEquals(k, v) => doc.meta(k).iter().any(|x| x.trim().eq_ignore_ascii_case(v.trim())),
            }
        };
        (self.any.is_empty() || self.any.iter().any(eval)) && self.all.iter().all(eval)
    }
}

/// Rank confirmed candidates: exact host > wildcard, matched include > none, priority, then id.
pub fn rank(mut hits: Vec<(&str, UrlHit)>) -> Vec<(&str, UrlHit)> {
    hits.sort_by(|(ia, a), (ib, b)| {
        b.exact_host
            .cmp(&a.exact_host)
            .then(b.include_matched.cmp(&a.include_matched))
            .then(b.priority.cmp(&a.priority))
            .then(ia.cmp(ib))
    });
    hits
}

/// Distinct schema.org types kept per page for the fallback.
pub const MAX_DETECTED_TYPES: usize = 64;

/// schema.org types the page declares, in document order: JSON-LD @type, og:type, microdata itemtype.
pub fn detected_types(doc: &Document) -> Vec<String> {
    let mut out: Vec<String> = doc.jsonld().all_types();
    for og in doc.meta("og:type") {
        let base = og.trim().split(['.', ':']).next().unwrap_or("").to_string();
        let mut chars = base.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_uppercase().collect::<String>() + chars.as_str());
        }
    }
    for item in doc.microdata() {
        if let Some(t) = item.get("@type").and_then(|v| v.as_str()) {
            out.push(t.to_string());
        }
    }
    // A hostile page can declare thousands of types; the fallback only needs the first few distinct ones.
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.clone()));
    out.truncate(MAX_DETECTED_TYPES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(hosts: &[&str], include: &[&str], exclude: &[&str], priority: i32) -> CompiledMatch {
        let m = pb::RecipeMatch {
            urls: Some(pb::UrlMatch {
                hosts: hosts.iter().map(|s| s.to_string()).collect(),
                include: include.iter().map(|s| s.to_string()).collect(),
                exclude: exclude.iter().map(|s| s.to_string()).collect(),
            }),
            page: None,
            priority,
        };
        CompiledMatch::compile(&m, &Limits::default()).unwrap()
    }

    #[test]
    fn www_is_an_alias_and_exclude_wins() {
        // Arrange
        let m = matcher(&["wsj.com"], &["^/articles/"], &["/print$"], 0);

        // Act
        let hit = m.match_url(&Url::parse("https://www.wsj.com/articles/x").unwrap());
        let printed = m.match_url(&Url::parse("https://www.wsj.com/articles/x/print").unwrap());

        // Assert
        assert_eq!(
            hit,
            Some(UrlHit {
                exact_host: true,
                include_matched: true,
                priority: 0
            })
        );
        assert!(printed.is_none());
    }

    #[test]
    fn exact_host_outranks_wildcard() {
        // Arrange
        let blog = matcher(&["blog.example.com"], &["^/20\\d\\d/"], &[], 1);
        let site = matcher(&["example.com", "*.example.com"], &[], &[], 0);
        let url = Url::parse("https://blog.example.com/2026/10/x").unwrap();
        let hits = vec![
            ("example.com/main", site.match_url(&url).unwrap()),
            ("example.com/blog", blog.match_url(&url).unwrap()),
        ];

        // Act
        let ranked = rank(hits);

        // Assert
        assert_eq!(ranked[0].0, "example.com/blog");
    }

    #[test]
    fn page_stage_and_type_detection() {
        // Arrange
        let doc = Document::parse(
            r#"<meta property="og:type" content="article"><script type="application/ld+json">{"@type":"BlogPosting"}</script><table><tr class=athing></tr></table>"#,
            "https://x.example/p",
            &Limits::default(),
        )
        .unwrap();
        let m = pb::RecipeMatch {
            urls: None,
            page: Some(pb::PageMatch {
                any: vec![pb::PageCondition {
                    condition: Some(pb::page_condition::Condition::JsonLdType(pb::JsonLdTypes {
                        types: vec!["NewsArticle".into()],
                    })),
                }],
                all: vec![pb::PageCondition {
                    condition: Some(pb::page_condition::Condition::Exists("tr.athing".into())),
                }],
            }),
            priority: 0,
        };
        let compiled = CompiledMatch::compile(&m, &Limits::default()).unwrap();

        // Act
        let confirmed = compiled.match_page(&doc, &Budget::new(10_000));
        let types = detected_types(&doc);

        // Assert
        assert!(!confirmed, "any-condition (NewsArticle) does not hold");
        assert_eq!(types, vec!["BlogPosting".to_string(), "Article".to_string()]);
    }
}
