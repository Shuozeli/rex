//! Automatic quality gate for generated recipes (decisions.md U31): what a reviewer would check first,
//! run on unredacted captures of real pages. Used by `rex draft` (repair feedback) and `rex gate`.
//!
//! Positive pages (pages of the kind the recipe is for) must be matched and chosen, extract with status OK,
//! and produce plausible values: for articles a title that agrees with the page's own metadata and a
//! substantial body; for feeds enough items with titles and links. Negative pages (for example the site's
//! homepage for an article recipe) must not be matched.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use rex_dom::{Document, Limits};
use rex_engine::{Ctx, Engine};
use scraper::{Html, Selector};
use serde::Serialize;

pub const ARTICLE: &str = "rex.schema.article.v1.Article";
pub const FEED: &str = "rex.schema.feed.v1.Feed";
const MIN_BODY_CHARS: usize = 300;
const MIN_AGREEMENT_BPS: i64 = 5000;
const MIN_TITLE_OVERLAP: f64 = 0.6;
const MIN_FEED_ITEMS: usize = 5;
/// A page whose readable text is shorter than this is a stub (paywall teaser, video page, live blog shell):
/// it must still match, but its body is not judged. At least `MIN_FULL_PAGES` full pages must pass.
const STUB_CHARS: usize = 600;
const MIN_FULL_PAGES: usize = 2;

/// One captured page.
pub struct Page {
    pub label: String,
    pub url: String,
    pub html: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PageOutcome {
    pub label: String,
    pub matched: bool,
    pub status: String,
    /// Too little readable text to judge the body (see STUB_CHARS).
    pub stub: bool,
    pub problems: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub pass: bool,
    pub pages: Vec<PageOutcome>,
}

/// Label of the outcome about the sample as a whole (too few full pages), which no recipe change can fix.
pub const SAMPLE_LABEL: &str = "sample";

impl Report {
    /// True when a page itself has problems, i.e. a changed recipe could pass (not only the sample count).
    pub fn recipe_fixable(&self) -> bool {
        self.pages
            .iter()
            .any(|p| p.label != SAMPLE_LABEL && !p.problems.is_empty())
    }

    pub fn problems(&self) -> Vec<String> {
        self.pages
            .iter()
            .flat_map(|p| p.problems.iter().map(move |x| format!("{}: {x}", p.label)))
            .collect()
    }
}

/// Words (alphanumeric runs in any script, lowercased) plus every CJK character as its own token.
fn tokens(s: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut word = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() && !rex_dom::text::is_cjk(c) {
            word.extend(c.to_lowercase());
            continue;
        }
        if !word.is_empty() {
            out.insert(std::mem::take(&mut word));
        }
        if rex_dom::text::is_cjk(c) {
            out.insert(c.to_string());
        }
    }
    if !word.is_empty() {
        out.insert(word);
    }
    out
}

/// |A ∩ B| / min(|A|, |B|): a site suffix on one side ("Title | Site") does not count against it.
#[cfg(test)]
fn overlap(a: &str, b: &str) -> f64 {
    overlap_without(a, b, &BTreeSet::new())
}

/// [`overlap`] after dropping the site-name tokens from both sides, so a title that is only the site name
/// does not match "Headline | Site" through the suffix.
fn overlap_without(a: &str, b: &str, site: &BTreeSet<String>) -> f64 {
    let (mut ta, mut tb) = (tokens(a), tokens(b));
    ta.retain(|t| !site.contains(t));
    tb.retain(|t| !site.contains(t));
    let min = ta.len().min(tb.len());
    if min == 0 {
        return 0.0;
    }
    ta.intersection(&tb).count() as f64 / min as f64
}

/// Titles the page states about itself: og:title, twitter:title, JSON-LD headline/name, <title>.
fn page_titles(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);
    let mut out = Vec::new();
    for sel in [
        r#"meta[property="og:title"]"#,
        r#"meta[name="twitter:title"]"#,
        r#"meta[property="twitter:title"]"#,
    ] {
        let s = Selector::parse(sel).expect("static selector");
        out.extend(
            doc.select(&s)
                .filter_map(|m| m.value().attr("content").map(str::to_string)),
        );
    }
    let ld = Selector::parse(r#"script[type="application/ld+json"]"#).expect("static selector");
    for block in doc.select(&ld) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&block.text().collect::<String>()) {
            collect_headlines(&v, &mut out);
        }
    }
    let title = Selector::parse("title").expect("static selector");
    out.extend(doc.select(&title).map(|t| t.text().collect::<String>()));
    out.retain(|t| !t.trim().is_empty());
    out
}

/// Tokens of the names the page gives its site: og:site_name, application-name, JSON-LD publisher.name.
fn site_tokens(html: &str) -> BTreeSet<String> {
    let doc = Html::parse_document(html);
    let mut names = Vec::new();
    for sel in [r#"meta[property="og:site_name"]"#, r#"meta[name="application-name"]"#] {
        let s = Selector::parse(sel).expect("static selector");
        names.extend(
            doc.select(&s)
                .filter_map(|m| m.value().attr("content").map(str::to_string)),
        );
    }
    let ld = Selector::parse(r#"script[type="application/ld+json"]"#).expect("static selector");
    for block in doc.select(&ld) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&block.text().collect::<String>()) {
            collect_publishers(&v, &mut names);
        }
    }
    names.iter().flat_map(|n| tokens(n)).collect()
}

fn collect_publishers(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(name) = m.get("publisher").and_then(|p| p.get("name")).and_then(|n| n.as_str()) {
                out.push(name.to_string());
            }
            if let Some(g) = m.get("@graph") {
                collect_publishers(g, out);
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_publishers(x, out)),
        _ => {}
    }
}

fn collect_headlines(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(serde_json::Value::String(h)) = m.get("headline") {
                out.push(h.clone());
            }
            for (k, x) in m {
                if k == "@graph" || k == "mainEntity" || k == "mainEntityOfPage" {
                    collect_headlines(x, out);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_headlines(x, out)),
        _ => {}
    }
}

fn str_field<'a>(v: &'a serde_json::Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn check_article(record: &serde_json::Value, ext: &serde_json::Value, html: &str, problems: &mut Vec<String>) {
    let title = str_field(record, "title");
    if title.trim().is_empty() {
        problems.push("title is empty".into());
    } else {
        let stated = page_titles(html);
        let site = site_tokens(html);
        let best = stated
            .iter()
            .map(|t| overlap_without(title, t, &site))
            .fold(0.0, f64::max);
        if !site.is_empty() && tokens(title).is_subset(&site) {
            problems.push(format!(
                "title {:?} is only the site's name",
                title.chars().take(80).collect::<String>()
            ));
        } else if !stated.is_empty() && best < MIN_TITLE_OVERLAP {
            problems.push(format!(
                "title {:?} does not match the page's own title {:?} (overlap {best:.2})",
                title.chars().take(80).collect::<String>(),
                stated[0].chars().take(80).collect::<String>()
            ));
        }
    }
    let body = str_field(record, "bodyText");
    let chars = body.chars().count();
    if chars < MIN_BODY_CHARS {
        problems.push(format!("body_text has {chars} characters (want >= {MIN_BODY_CHARS})"));
    }
    if let Some(bps) = ext.get("bodyReadabilityAgreementBps").and_then(|x| x.as_i64()) {
        if bps < MIN_AGREEMENT_BPS {
            problems.push(format!(
                "body agrees with the readability extraction at only {}% (want >= {}%): the body selector \
                 probably picks the wrong container or misses most of the text",
                bps / 100,
                MIN_AGREEMENT_BPS / 100
            ));
        }
    }
}

fn check_feed(record: &serde_json::Value, problems: &mut Vec<String>) {
    let items = record
        .get("items")
        .and_then(|x| x.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if items.len() < MIN_FEED_ITEMS {
        problems.push(format!("{} items (want >= {MIN_FEED_ITEMS})", items.len()));
        return;
    }
    let complete = items
        .iter()
        .filter(|i| !str_field(i, "title").trim().is_empty() && !str_field(i, "url").is_empty())
        .count();
    if complete * 5 < items.len() * 4 {
        problems.push(format!(
            "only {complete} of {} items have both a title and a url",
            items.len()
        ));
    }
}

/// Why recipe `id` did not match a page: which stage rejected it, with the rules it applied.
fn match_diagnosis(engine: &Engine, id: &str, url: &str, url_hit: bool) -> String {
    let rules = engine
        .recipes()
        .get(id)
        .and_then(|r| rex_engine::recipe::canonical_json(r, engine.recipes().registry()).ok())
        .and_then(|(json, _)| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|v| v.get("match").cloned())
        .unwrap_or_default();
    if url_hit {
        format!(
            "the URL matched but the page conditions failed on this page: match.page = {} (use a condition every \
             page of this kind has, e.g. a JSON-LD @type, og:type, or an element present on all of them)",
            rules.get("page").map(|v| v.to_string()).unwrap_or_else(|| "{}".into())
        )
    } else {
        let path = url::Url::parse(url)
            .map(|u| format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()))
            .unwrap_or_default();
        format!(
            "the URL rules reject this page's path {path:?}: match.urls = {} (include/exclude are RE2 regexes \
             on path+query; hosts must cover {:?})",
            rules.get("urls").map(|v| v.to_string()).unwrap_or_else(|| "{}".into()),
            url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_default()
        )
    }
}

/// Run the gate for recipe `id` (already compiled into `engine`).
pub fn check(
    engine: &Engine,
    id: &str,
    schema: &str,
    positives: &[Page],
    negatives: &[Page],
    now: DateTime<Utc>,
) -> Report {
    let limits: &Limits = engine.limits();
    let ctx = Ctx {
        now,
        fetch_time: Some(now),
        hints: BTreeMap::new(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "rex-gate".into(),
    };
    let mut pages = Vec::new();
    for p in positives {
        let mut problems = Vec::new();
        let doc = match Document::parse(&p.html, &p.url, limits) {
            Ok(d) => d,
            Err(e) => {
                pages.push(PageOutcome {
                    label: p.label.clone(),
                    matched: false,
                    status: String::new(),
                    stub: false,
                    problems: vec![format!("page does not parse: {e}")],
                });
                continue;
            }
        };
        let plan = engine.plan(&doc);
        let matched = plan.confirmed.iter().any(|c| c == id);
        if !matched {
            problems.push(match_diagnosis(
                engine,
                id,
                &p.url,
                plan.url_hits.iter().any(|(h, _)| h == id),
            ));
        } else if plan.chosen.as_deref() != Some(id) {
            problems.push(format!("matched, but {:?} is chosen instead", plan.chosen));
        }
        let stub = schema == ARTICLE
            && rex_readability::extract(&doc, &rex_dom::Budget::new(limits.fuel))
                .map(|r| r.content_text.chars().count() < STUB_CHARS)
                .unwrap_or(false);
        let mut status = String::new();
        match engine.extract(id, &doc, &ctx) {
            None => problems.push("the recipe did not run".into()),
            Some(r) => {
                let ext = engine.to_json(&r.extraction);
                let record = engine.record_json(&r.record);
                status = str_field(&ext, "status")
                    .trim_start_matches("EXTRACTION_STATUS_")
                    .to_string();
                if status != "OK" && !stub {
                    let missing = ext.get("missingRequired").map(|m| m.to_string()).unwrap_or_default();
                    let failed = ext.get("expectationsFailed").map(|m| m.to_string()).unwrap_or_default();
                    problems.push(format!(
                        "status {status} (missing required {missing}, expectations failed {failed})"
                    ));
                }
                match schema {
                    ARTICLE if stub => {}
                    ARTICLE => check_article(&record, &ext, &p.html, &mut problems),
                    FEED => check_feed(&record, &mut problems),
                    _ => {}
                }
            }
        }
        pages.push(PageOutcome {
            label: p.label.clone(),
            matched,
            status,
            stub,
            problems,
        });
    }
    let full = pages.iter().filter(|o| !o.stub).count();
    if full < MIN_FULL_PAGES {
        pages.push(PageOutcome {
            label: SAMPLE_LABEL.into(),
            matched: false,
            status: String::new(),
            stub: false,
            problems: vec![format!(
                "only {full} of the sample pages have enough readable text to judge (want >= {MIN_FULL_PAGES})"
            )],
        });
    }
    for p in negatives {
        let matched = Document::parse(&p.html, &p.url, limits)
            .map(|doc| engine.plan(&doc).confirmed.iter().any(|c| c == id))
            .unwrap_or(false);
        pages.push(PageOutcome {
            label: p.label.clone(),
            matched,
            status: String::new(),
            stub: false,
            problems: if matched {
                vec!["this page must NOT match (it is not a page of this kind); tighten match.urls.include or match.page".into()]
            } else {
                Vec::new()
            },
        });
    }
    let pass = pages.iter().all(|p| p.problems.is_empty());
    Report { pass, pages }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_ignores_a_site_suffix() {
        // Arrange
        let extracted = "Go 1.22 is released!";
        let stated = "Go 1.22 is released! - The Go Programming Language";

        // Act
        let o = overlap(extracted, stated);

        // Assert
        assert!((o - 1.0).abs() < 1e-9, "{o}");
    }

    #[test]
    fn overlap_counts_cjk_characters() {
        // Arrange
        let extracted = "科技爱好者周刊（第 414 期）";
        let stated = "科技爱好者周刊（第 414 期）：AI 的未来 - 阮一峰的网络日志";

        // Act
        let o = overlap(extracted, stated);

        // Assert
        assert!(o > 0.99, "{o}");
    }

    #[test]
    fn overlap_tokenizes_accented_and_cyrillic_words() {
        // Arrange
        let extracted = "Économie: la croissance ralentit";
        let stated = "économie : la croissance ralentit — Le Monde";

        // Act
        let o = overlap(extracted, stated);

        // Assert
        assert_eq!(o, 1.0);
        assert!(tokens("Москва сегодня").contains("москва"));
    }

    #[test]
    fn a_title_that_is_only_the_site_name_is_rejected() {
        // Arrange
        let html = r#"<html><head><title>Rates rise again | The Daily</title><meta property="og:site_name" content="The Daily"></head><body></body></html>"#;
        let record = serde_json::json!({ "title": "The Daily", "bodyText": "x".repeat(2000) });
        let mut problems = Vec::new();

        // Act
        check_article(&record, &serde_json::json!({}), html, &mut problems);

        // Assert
        assert!(
            problems.iter().any(|p| p.contains("only the site's name")),
            "{problems:?}"
        );
    }

    #[test]
    fn unrelated_titles_do_not_overlap() {
        // Arrange
        let extracted = "Subscribe to our newsletter";
        let stated = "Fed holds rates steady as inflation cools";

        // Act
        let o = overlap(extracted, stated);

        // Assert
        assert!(o < 0.2, "{o}");
    }

    #[test]
    fn page_titles_reads_og_jsonld_and_title() {
        // Arrange
        let html = r#"<html><head><title>T1</title><meta property="og:title" content="T2"><script type="application/ld+json">{"@graph":[{"@type":"NewsArticle","headline":"T3"}]}</script></head></html>"#;

        // Act
        let t = page_titles(html);

        // Assert
        assert_eq!(t, vec!["T2", "T3", "T1"]);
    }
}
