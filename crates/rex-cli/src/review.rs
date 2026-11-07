//! `rex judge` (decisions.md U44, layer 3): an LLM reads the page evidence and the extracted record and reports
//! values a careful reader would call wrong (internal codes as tags, a site name in the title, a teaser as the
//! body, list items shifted by one, ...). Layer 1 (`rex_engine::judge`) catches the mechanical cases with
//! fixed rules; this catches the ones that need reading the page. `rex draft` sends its error findings back to
//! the model as a repair round, and the periodic audit runs it over every captured page.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use rex_dom::{Budget, Document, Limits};
use rex_schema::{RecordSpec, SchemaRegistry};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};

use crate::draft::{describe_record, extract_json, short, Llm, Role, Turn};

/// Bump when the rubric or the evidence changes: cached verdicts from older prompts are not reused.
const PROMPT_VERSION: u32 = 2;
const MAX_ATTEMPTS: usize = 2;
const RECORD_BUDGET: usize = 14_000;
const TEXT_BUDGET: usize = 3_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Finding {
    /// Record path, e.g. `tags`, `tags[2]`, `items[4].title`, or `missing:<field>`.
    pub field: String,
    pub severity: Severity,
    pub problem: String,
    /// What on the page shows it: a verbatim quote from the evidence. An error whose quote is not in the evidence is
    /// downgraded to a warning (see [`ground`]).
    #[serde(default)]
    pub evidence: String,
    /// What the value should be, or where it should come from.
    #[serde(default)]
    pub fix: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level} {}: {}", self.field, self.problem)?;
        if !self.fix.is_empty() {
            write!(f, " (fix: {})", self.fix)?;
        }
        Ok(())
    }
}

#[derive(serde::Deserialize)]
struct Reply {
    findings: Vec<Finding>,
}

/// One extraction to review.
pub struct Case<'a> {
    pub url: &'a str,
    pub html: &'a str,
    pub schema: &'a RecordSpec,
    pub registry: &'a SchemaRegistry,
    /// The extraction output (`Engine::to_json`): its `record`, and `sources` saying which recipe filled what.
    pub extraction: &'a serde_json::Value,
}

pub fn system_prompt() -> String {
    "You review the output of rex, a tool that turns one web page into a typed record. You get evidence from the \
     page and the record extracted from it. Report every value a careful person reading the page would call \
     wrong, and nothing else.\n\n\
     Wrong values (severity \"error\"):\n\
     - tags / keywords / genres / categories: each must be a topic label a reader would recognize for this item. \
       Errors: internal codes, ids, tracking or ad keys, CMS taxonomy paths or prefixed keys (\"SYND\", \"LINK|...\", \
       \"l1:politics\", \"n/elx\", \"wc_1234\"), the site's own navigation sections dumped as tags, the same tag \
       twice in different case.\n\
     - title / name / headline: the item's own title. Error: the site name, section or a \"| Site\" / \"- Site\" \
       suffix included; the title of another item; a generic page heading instead of the item's.\n\
     - authors / creators: names of people or organizations only. Errors: \"By\", roles, dates, URLs or emails \
       inside the name; navigation text. The site's name standing in for the author is a warning.\n\
     - dates: the item's own publication/update time as declared on the page. Errors: in the future relative to \
       the fetch time, a copyright year, the date of a different item, the fetch time itself.\n\
     - summary / description: the item's own description. Error: the site-wide description, cookie, paywall or \
       sign-up text.\n\
     - body: the main content. Errors: navigation, comments, related links or paywall prompts as the body; a teaser \
       when the page has the full text.\n\
     - lists of items (results, posts, products, comments): each entry is a real entry of the page's list. Errors: \
       ads, navigation links or headers as entries; titles that do not belong to their URL; fields shifted from a \
       neighboring entry; the same entry repeated.\n\
     - numbers, prices, currencies, counts, ratings: must match the page, with the right unit and currency.\n\
     - text: HTML entities (&amp; &#x27;), markup or scripts inside a plain-text value.\n\n\
     Warnings (severity \"warning\"): a field the evidence clearly shows but the record lacks (field \
     \"missing:<name>\"); a value that is plausible but probably not the best one on the page.\n\n\
     The record is protobuf JSON: field names are lowerCamelCase, 64-bit integers are JSON strings (\"45327\"), \
     enums are full names (AVAILABILITY_IN_STOCK), timestamps are RFC 3339 UTC, and empty fields are left out. \
     These formats are never errors; judge the values.\n\n\
     Do not report: an item listed twice because the page lists it twice (a sponsored slot and the same product \
     among the organic results); fewer items than the page will show after scrolling (placeholders not yet \
     rendered are not items); numbers you matched to the wrong neighbor (a card's text runs \"<plays> <comments> \
     <duration> <title>\": read the order the page uses before calling a value wrong); anything you cannot verify \
     from the evidence (it is partial: a value missing from the excerpt \
     is not proof); style preferences; text cut at a length limit; redacted text (test pages replace private text \
     with runs of x, X or 字, so names and sentences may read \"Xxxx Xxxxxx\").\n\n\
     Every error needs proof: `evidence` is a short passage (under 150 characters) copied character for character \
     from the page evidence above (meta tags, JSON-LD, text), not from the record and not paraphrased. A finding you \
     cannot prove that way is at most a warning. Before reporting a mismatch, check that the two values really differ \
     (\"2026-10-10T17:47:03-04:00\" and \"2026-10-10T21:47:03Z\" are the same time; \"12.2K\" and \"12200\" are the \
     same count).\n\n\
     Reply with exactly one JSON object and nothing else: {\"findings\": [{\"field\": \"<record path, e.g. tags[2] or \
     items[4].title>\", \"severity\": \"error\"|\"warning\", \"problem\": \"<what is wrong>\", \"evidence\": \"<verbatim \
     quote from the page evidence>\", \"fix\": \"<the right value or where it is on the page>\"}]}. An empty list \
     means the record is right."
        .to_string()
}

/// The user message: the page evidence, the schema and the record.
pub fn prompt(case: &Case) -> Result<String> {
    let html = Html::parse_document(case.html);
    let mut out = format!("## Page\nURL: {}\n", case.url);
    let title = Selector::parse("title").expect("static selector");
    if let Some(t) = html.select(&title).next() {
        out.push_str(&format!("<title>: {}\n", short(&t.text().collect::<String>(), 200)));
    }
    let h1 = Selector::parse("h1").expect("static selector");
    for h in html.select(&h1).take(3) {
        out.push_str(&format!("<h1>: {}\n", short(&h.text().collect::<String>(), 200)));
    }

    let meta = Selector::parse("meta[content]").expect("static selector");
    out.push_str("\n## Meta tags\n");
    for m in html.select(&meta).take(60) {
        let v = m.value();
        if let Some(k) = v
            .attr("property")
            .or_else(|| v.attr("name"))
            .or_else(|| v.attr("itemprop"))
        {
            out.push_str(&format!(
                "- {k} = {}\n",
                short(v.attr("content").unwrap_or_default(), 200)
            ));
        }
    }

    let ld = Selector::parse(r#"script[type="application/ld+json"]"#).expect("static selector");
    let blocks: Vec<String> = html.select(&ld).map(|s| s.text().collect::<String>()).collect();
    out.push_str(&format!("\n## JSON-LD ({} blocks)\n", blocks.len()));
    for b in blocks.iter().take(3) {
        let compact = serde_json::from_str::<serde_json::Value>(b)
            .ok()
            .and_then(|v| serde_json::to_string(&v).ok())
            .unwrap_or_else(|| b.trim().to_string());
        out.push_str(&short(&compact, 2_500));
        out.push('\n');
    }

    let limits = Limits::default();
    if let Ok(doc) = Document::parse(case.html, case.url, &limits) {
        if let Ok(r) = rex_readability::extract(&doc, &Budget::new(limits.fuel)) {
            out.push_str("\n## Main text (readability excerpt)\n");
            if !r.byline.is_empty() {
                out.push_str(&format!("Byline: {}\n", short(&r.byline, 200)));
            }
            out.push_str(&short(&r.content_text, TEXT_BUDGET));
            out.push('\n');
        }
    }
    out.push_str("\n## Visible text of the page (start)\n");
    let body = Selector::parse("body").expect("static selector");
    if let Some(b) = html.select(&body).next() {
        out.push_str(&short(&visible_text(b), TEXT_BUDGET));
        out.push('\n');
    }

    out.push_str(&format!("\n## Schema {}\n", case.schema.full_name()));
    describe_record(case.registry, case.schema, 0, &mut out)?;

    out.push_str("\n## Where each field came from (path <- origin, recipe)\n");
    for s in case
        .extraction
        .get("sources")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .take(80)
    {
        let field = |k: &str| s.get(k).and_then(|v| v.as_str()).unwrap_or_default();
        out.push_str(&format!(
            "- {} <- {} ({})\n",
            field("path"),
            field("origin").trim_start_matches("FIELD_ORIGIN_"),
            field("recipe")
        ));
    }

    let record = case
        .extraction
        .get("record")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let pretty = serde_json::to_string_pretty(&record)?;
    out.push_str("\n## Extracted record\n");
    if pretty.chars().count() > RECORD_BUDGET {
        out.push_str(&pretty.chars().take(RECORD_BUDGET).collect::<String>());
        out.push_str("\n(record truncated)\n");
    } else {
        out.push_str(&pretty);
        out.push('\n');
    }
    out.push_str("\nReturn only the JSON object.\n");
    Ok(out)
}

/// Text outside scripts, styles and templates, whitespace collapsed.
fn visible_text(e: scraper::ElementRef) -> String {
    let mut out = String::new();
    for node in e.descendants() {
        if let Some(t) = node.value().as_text() {
            let hidden = node
                .ancestors()
                .filter_map(scraper::ElementRef::wrap)
                .any(|a| matches!(a.value().name(), "script" | "style" | "noscript" | "template" | "svg"));
            if !hidden {
                out.push_str(t);
                out.push(' ');
            }
        }
        if out.len() > TEXT_BUDGET * 4 {
            break;
        }
    }
    out
}

/// Review one extraction. Unparseable replies are retried once.
pub fn review(llm: &dyn Llm, case: &Case) -> Result<Vec<Finding>> {
    review_prompt(llm, &prompt(case)?)
}

/// Review from a prompt built by [`prompt`] (built on the extraction thread, sent from any thread).
pub fn review_prompt(llm: &(impl Llm + ?Sized), user: &str) -> Result<Vec<Finding>> {
    let system = system_prompt();
    let mut turns = vec![Turn {
        role: Role::User,
        text: user.to_string(),
    }];
    let mut attempt = 0;
    loop {
        attempt += 1;
        let reply = llm.complete(&system, &turns)?;
        match parse(&reply) {
            Ok(findings) => return Ok(ground(findings, user)),
            Err(e) if attempt < MAX_ATTEMPTS => {
                turns.push(Turn {
                    role: Role::Assistant,
                    text: reply,
                });
                turns.push(Turn {
                    role: Role::User,
                    text: format!("That reply is invalid: {e:#}\nReturn only the JSON object {{\"findings\": [...]}}."),
                });
            }
            Err(e) => bail!("no valid review after {MAX_ATTEMPTS} attempts: {e:#}"),
        }
    }
}

/// Errors must quote the evidence: one whose `evidence` (whitespace and quotes normalized) is not a passage of the
/// prompt's page evidence is a warning, marked "(unproven)". This keeps the model's misreadings (a time zone it did not
/// convert, a product it did not find) from failing a recipe.
pub fn ground(findings: Vec<Finding>, user: &str) -> Vec<Finding> {
    let page = user.split("\n## Schema ").next().unwrap_or(user);
    let haystack = squash(page);
    findings
        .into_iter()
        .map(|mut f| {
            let quote = squash(&f.evidence);
            let proven = quote.chars().count() >= 3
                && quote
                    .split('…')
                    .chain(quote.split("..."))
                    .filter(|part| part.chars().count() >= 3)
                    .all(|part| haystack.contains(part.trim()));
            if f.severity == Severity::Error && !proven {
                f.severity = Severity::Warning;
                f.problem = format!("(unproven) {}", f.problem);
            }
            f
        })
        .collect()
}

/// Lowercased, whitespace collapsed, typographic quotes made plain, JSON escapes of quotes removed.
fn squash(s: &str) -> String {
    let plain: String = s
        .replace("\\\"", "\"")
        .chars()
        .map(|c| match c {
            '“' | '”' | '„' => '"',
            '‘' | '’' => '\'',
            c => c,
        })
        .collect();
    plain.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

pub fn parse(reply: &str) -> Result<Vec<Finding>> {
    let r: Reply = serde_json::from_str(extract_json(reply)?).map_err(|e| anyhow!("{e}"))?;
    Ok(r.findings)
}

/// Verdicts kept on disk, keyed by the model and the whole prompt: a page and recipe that did not change are not
/// sent again.
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self { dir: dir.to_path_buf() })
    }

    fn path(&self, llm: &(impl Llm + ?Sized), user: &str) -> PathBuf {
        let mut h = Sha256::new();
        h.update(PROMPT_VERSION.to_le_bytes());
        h.update(llm.describe().as_bytes());
        h.update(system_prompt().as_bytes());
        h.update(user.as_bytes());
        let key: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{key}.json"))
    }

    /// The cached verdict for a prompt built by [`prompt`], or a fresh one (stored); `true` = from the cache.
    pub fn review(&self, llm: &(impl Llm + ?Sized), user: &str) -> Result<(Vec<Finding>, bool)> {
        let path = self.path(llm, user);
        if let Some(found) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Finding>>(&s).ok())
        {
            return Ok((found, true));
        }
        let found = review_prompt(llm, user)?;
        std::fs::write(&path, serde_json::to_string_pretty(&found)?)?;
        Ok((found, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeLlm {
        replies: RefCell<Vec<String>>,
        calls: RefCell<usize>,
    }

    impl FakeLlm {
        fn new(replies: &[&str]) -> Self {
            Self {
                replies: RefCell::new(replies.iter().rev().map(|s| s.to_string()).collect()),
                calls: RefCell::new(0),
            }
        }
    }

    impl Llm for FakeLlm {
        fn describe(&self) -> String {
            "fake".into()
        }
        fn complete(&self, _system: &str, _turns: &[Turn]) -> Result<String> {
            *self.calls.borrow_mut() += 1;
            self.replies
                .borrow_mut()
                .pop()
                .ok_or_else(|| anyhow!("no more replies"))
        }
    }

    const PAGE: &str = r#"<html><head><title>Fed holds rates | WSJ</title>
        <meta name="keywords" content="SYND,LINK|1234,Interest Rates">
        <script type="application/ld+json">{"@type":"NewsArticle","headline":"Fed holds rates"}</script></head>
        <body><h1>Fed holds rates</h1><p>The Federal Reserve kept rates unchanged on Wednesday.</p>
        <script>var hidden = "do not show";</script></body></html>"#;

    fn extraction() -> serde_json::Value {
        serde_json::json!({
            "record": {"title": "Fed holds rates", "tags": ["SYND", "LINK|1234", "Interest Rates"]},
            "sources": [{"path": "tags", "origin": "FIELD_ORIGIN_META", "recipe": "generic/article.v1@1"}]
        })
    }

    fn with_case<T>(f: impl FnOnce(&Case) -> T) -> T {
        let registry = SchemaRegistry::embedded().unwrap();
        let schema = registry.record("rex.schema.article.v1.Article").unwrap();
        let ext = extraction();
        let case = Case {
            url: "https://www.wsj.com/economy/fed-holds-rates-4f2a1b3c",
            html: PAGE,
            schema: &schema,
            registry: &registry,
            extraction: &ext,
        };
        f(&case)
    }

    #[test]
    fn prompt_carries_page_evidence_sources_and_record_but_not_scripts() {
        // Arrange / Act
        let p = with_case(|c| prompt(c).unwrap());

        // Assert
        assert!(p.contains("<title>: Fed holds rates | WSJ"));
        assert!(p.contains("- keywords = SYND,LINK|1234,Interest Rates"));
        assert!(p.contains("\"headline\":\"Fed holds rates\""));
        assert!(p.contains("- tags <- META (generic/article.v1@1)"));
        assert!(p.contains("\"LINK|1234\""));
        assert!(p.contains("The Federal Reserve kept rates unchanged"));
        assert!(!p.contains("do not show"));
    }

    #[test]
    fn findings_are_parsed_from_a_fenced_reply() {
        // Arrange
        let llm = FakeLlm::new(&[
            "```json\n{\"findings\": [{\"field\": \"tags[0]\", \"severity\": \"error\", \"problem\": \"internal code\", \"evidence\": \"SYND,LINK|1234\"}]}\n```",
        ]);

        // Act
        let found = with_case(|c| review(&llm, c).unwrap());

        // Assert
        assert_eq!(
            found,
            vec![Finding {
                field: "tags[0]".into(),
                severity: Severity::Error,
                problem: "internal code".into(),
                evidence: "SYND,LINK|1234".into(),
                fix: String::new(),
            }]
        );
    }

    #[test]
    fn an_error_without_a_verbatim_quote_is_only_a_warning() {
        // Arrange
        let user = with_case(|c| prompt(c).unwrap());
        let finding = |evidence: &str| Finding {
            field: "tags[0]".into(),
            severity: Severity::Error,
            problem: "internal code".into(),
            evidence: evidence.into(),
            fix: String::new(),
        };
        let found = vec![
            finding("keywords = SYND,LINK|1234"),
            finding("the page lists different tags"),
            finding(""),
            // In the record only, not in the page evidence:
            finding("\"tags\": ["),
        ];

        // Act
        let grounded = ground(found, &user);

        // Assert
        let levels: Vec<Severity> = grounded.iter().map(|f| f.severity).collect();
        assert_eq!(
            levels,
            [Severity::Error, Severity::Warning, Severity::Warning, Severity::Warning]
        );
        assert!(grounded[1].problem.starts_with("(unproven)"));
    }

    #[test]
    fn an_unparseable_reply_is_retried_once_then_fails() {
        // Arrange
        let llm = FakeLlm::new(&["looks fine to me", "{\"verdict\": \"ok\"}"]);

        // Act
        let result = with_case(|c| review(&llm, c));

        // Assert
        assert!(result.is_err());
        assert_eq!(*llm.calls.borrow(), 2);
    }

    #[test]
    fn a_cached_verdict_is_not_asked_again() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path()).unwrap();
        let llm = FakeLlm::new(&["{\"findings\": []}"]);
        let user = with_case(|c| prompt(c).unwrap());
        cache.review(&llm, &user).unwrap();

        // Act
        let (found, cached) = cache.review(&llm, &user).unwrap();

        // Assert
        assert!(found.is_empty());
        assert!(cached);
        assert_eq!(*llm.calls.borrow(), 1);
    }
}
