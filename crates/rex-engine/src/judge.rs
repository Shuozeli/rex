//! Field-quality rules for an extracted record (decisions.md U44, layer 1 of the recipe judge).
//!
//! The status says whether a recipe found what the schema requires; these rules say whether what it found
//! is any good. They are deterministic and schema-driven: each field's type (URL, timestamp, text, a list of
//! people…) and name decide which rules apply. `rex test` runs them on every fixture, `rex liveness` on live
//! pages and `rex draft` in its gate; an `Error` fails the case.
//!
//! Rules (the `rule` name is what a fixture's `judge_allow` names to accept a finding, with a reason):
//! - `tag-code`: a tag that is a publisher's internal code (`LINK|i13-…`, `l1:politics`, `n/elx`, `135740207`).
//! - `tag-duplicate`: the same tag twice, ignoring case (not judged on redacted text, where different words
//!   look alike).
//! - `tag-count` (warning): more than 20 tags (often a taxonomy dump; video tags can legitimately be many).
//! - `title-site`: a title or headline that ends with the site's name ("… | WSJ") or is only the site's name
//!   (a `name` may equal it: an app or a business named after its site).
//! - `person-junk`: a person's name with "By", a URL, an e-mail address, or over 80 characters.
//! - `text-entity`: an HTML entity left in text (`&amp;`, `&#39;`).
//! - `text-untrimmed`: leading or trailing whitespace.
//! - `time-future`: a publication or update time more than a day after the fetch (event times are future).
//! - `time-ancient`: a time before 1990 (a warning: archives exist).
//! - `url-relative`: a URL field that is not an absolute http(s) URL.
//! - `item-untitled` (warning): a list item without a title or name (an image-only reply has none).
//! - `item-duplicate` (warning): the same URL twice in a list of linked items (a page can list an item twice,
//!   e.g. sponsored and organic; steps that all link to their page are not linked items).
//! - `price-negative`: a negative price (zero is free).

use chrono::{DateTime, TimeZone, Utc};
use rex_schema::{RecordSpec, SchemaRegistry, StringKind, Target};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub level: Level,
    pub rule: &'static str,
    /// Record path in schema (snake_case) names, e.g. `tags[3]`, `authors[0].name`.
    pub path: String,
    pub detail: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
        };
        write!(f, "{level} {} at {}: {}", self.rule, self.path, self.detail)
    }
}

/// What the rules know about the page besides the record.
pub struct PageFacts {
    /// Names the site goes by: og:site_name, application-name, the host's registrable label ("wsj").
    pub site_names: Vec<String>,
    pub fetch_time: DateTime<Utc>,
}

impl PageFacts {
    pub fn of(doc: &rex_dom::Document, fetch_time: DateTime<Utc>) -> PageFacts {
        PageFacts {
            site_names: site_names(doc),
            fetch_time,
        }
    }
}

const SITE_SEPARATORS: &[&str] = &[" | ", " - ", " – ", " — ", " · ", " :: "];

/// Names the site goes by: og:site_name, application-name, twitter:site, each also without its tagline
/// ("少数派 - 高品质数字消费指南" -> "少数派"), and the host's registrable label ("wsj").
pub fn site_names(doc: &rex_dom::Document) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for key in ["og:site_name", "application-name", "twitter:site"] {
        for v in doc.meta(key) {
            let v = v.trim().trim_start_matches('@').to_string();
            for sep in SITE_SEPARATORS.iter().chain(["：", ": "].iter()) {
                if let Some((head, _)) = v.split_once(sep) {
                    names.push(head.trim().to_string());
                }
            }
            names.push(v);
        }
    }
    if let Some(host) = doc.url().host_str() {
        let parts: Vec<&str> = host.trim_start_matches("www.").split('.').collect();
        if parts.len() >= 2 {
            names.push(parts[parts.len() - 2].to_string());
        }
    }
    names.retain(|n| n.chars().count() >= 2);
    names.dedup();
    names
}

/// `title` without a trailing " | Site", " - Site", " · Site" naming the site itself (or "The Site"). A title that is
/// only the site's name is returned as it is.
pub fn strip_site_suffix(title: &str, site_names: &[String]) -> String {
    let names: Vec<String> = site_names
        .iter()
        .filter(|n| !looks_redacted(n))
        .map(|n| norm(n))
        .filter(|n| n.len() >= 2)
        .collect();
    let mut out = title.trim().to_string();
    // "Post - Blog | Site": up to two rounds, the outer one first.
    for _ in 0..2 {
        let cut = SITE_SEPARATORS.iter().find_map(|sep| {
            let (head, tail) = out.rsplit_once(sep)?;
            let tl = norm(tail);
            (!head.trim().is_empty() && names.iter().any(|n| tl == *n || tl == format!("the{n}")))
                .then(|| head.trim_end().to_string())
        });
        match cut {
            Some(head) => out = head,
            None => break,
        }
    }
    out
}

const TAG_FIELDS: &[&str] = &["tags", "keywords", "genres", "categories", "topics"];
const TITLE_FIELDS: &[&str] = &["title", "headline", "name"];
const MAX_TAGS: usize = 20;

/// Judge a record (protojson, as `Engine::record_json` writes it) against its schema.
pub fn judge(registry: &SchemaRegistry, schema: &str, record: &serde_json::Value, page: &PageFacts) -> Vec<Finding> {
    let mut out = Vec::new();
    if let Ok(spec) = registry.record(schema) {
        walk(registry, &spec, record, "", true, page, &mut out);
    }
    out
}

fn walk(
    registry: &SchemaRegistry,
    spec: &RecordSpec,
    value: &serde_json::Value,
    prefix: &str,
    top: bool,
    page: &PageFacts,
    out: &mut Vec<Finding>,
) {
    let Some(obj) = value.as_object() else { return };
    for field in &spec.fields {
        let Some(v) = obj.get(field.desc.json_name()) else {
            continue;
        };
        let path = if prefix.is_empty() {
            field.name.clone()
        } else {
            format!("{prefix}.{}", field.name)
        };
        let items: Vec<(String, &serde_json::Value)> = if field.repeated {
            v.as_array()
                .map(|a| a.iter().enumerate().map(|(i, x)| (format!("{path}[{i}]"), x)).collect())
                .unwrap_or_default()
        } else {
            vec![(path.clone(), v)]
        };

        if field.repeated && TAG_FIELDS.contains(&field.name.as_str()) {
            tags(&path, v, out);
        }
        if field.repeated {
            if let Target::Message(desc) = &field.target {
                list_items(&path, v, desc.full_name(), out);
            }
        }
        for (p, x) in items {
            match &field.target {
                Target::Str(kind) => {
                    if let Some(s) = x.as_str() {
                        string(&p, s, *kind, out);
                        if top && TITLE_FIELDS.contains(&field.name.as_str()) && !field.repeated {
                            title(&p, &field.name, s, page, out);
                        }
                        if field.name == "price" || field.name.ends_with("_price") {
                            price(&p, s, out);
                        }
                    }
                }
                Target::Timestamp => {
                    if let Some(s) = x.as_str() {
                        time(&p, &field.name, s, page, out);
                    }
                }
                Target::F32 | Target::F64 if field.name == "price" => {
                    if let Some(n) = x.as_f64() {
                        price(&p, &n.to_string(), out);
                    }
                }
                Target::Message(desc) => {
                    if let Ok(sub) = registry.record_of(desc) {
                        if is_person(desc.full_name()) {
                            person(&p, x, page, out);
                        }
                        walk(registry, &sub, x, &p, false, page, out);
                    }
                }
                _ => {}
            }
        }
    }
}

fn err(out: &mut Vec<Finding>, rule: &'static str, path: &str, detail: String) {
    out.push(Finding {
        level: Level::Error,
        rule,
        path: path.to_string(),
        detail,
    });
}

fn warn(out: &mut Vec<Finding>, rule: &'static str, path: &str, detail: String) {
    out.push(Finding {
        level: Level::Warning,
        rule,
        path: path.to_string(),
        detail,
    });
}

fn short(s: &str) -> String {
    let t: String = s.chars().take(60).collect();
    if t.len() < s.len() {
        format!("{t:?}…")
    } else {
        format!("{t:?}")
    }
}

/// A publisher's internal code rather than a topic a reader would recognize.
pub fn is_code(tag: &str) -> bool {
    let t = tag.trim();
    let bytes_digits = t.chars().filter(char::is_ascii_digit).count();
    t.contains('|')
        // "l1:politics", "LINK:EN", "drn:realtime"
        || (t.contains(':') && !t.contains(' ') && t.split(':').next().is_some_and(|p| p.chars().all(|c| c.is_ascii_alphanumeric())))
        // long numeric ids
        || (t.len() >= 5 && t.chars().all(|c| c.is_ascii_digit()))
        // "n/elx", "r/dc": a one- or two-letter lowercase namespace and a lowercase code ("N/A" is not one)
        || t.split_once('/').is_some_and(|(a, b)| {
            (1..=2).contains(&a.len())
                && a.chars().all(|c| c.is_ascii_lowercase())
                && (1..=8).contains(&b.len())
                && b.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
        // "c314", "i1013": a lowercase letter and at least three digits ("G20", "F35", "x86" are names)
        || (t.len() >= 4 && t.len() <= 7 && t.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && t.chars().skip(1).all(|c| c.is_ascii_digit()))
        // "wc_401_600", "gfx_contrib_2": snake_case with digits
        || (t.contains('_') && !t.contains(' ') && bytes_digits > 0)
}

fn tags(path: &str, v: &serde_json::Value, out: &mut Vec<Finding>) {
    let Some(arr) = v.as_array() else { return };
    let strs: Vec<&str> = arr.iter().filter_map(|x| x.as_str()).collect();
    let codes: Vec<&str> = strs.iter().copied().filter(|s| is_code(s)).collect();
    if !codes.is_empty() {
        err(
            out,
            "tag-code",
            path,
            format!(
                "{} of {} tags are internal codes, e.g. {}",
                codes.len(),
                strs.len(),
                codes.iter().take(4).map(|s| short(s)).collect::<Vec<_>>().join(", ")
            ),
        );
    }
    let mut seen = std::collections::HashSet::new();
    let dups: Vec<&str> = strs
        .iter()
        .copied()
        .filter(|s| !looks_redacted(s))
        .filter(|s| !seen.insert(s.to_lowercase()))
        .collect();
    if !dups.is_empty() {
        err(
            out,
            "tag-duplicate",
            path,
            format!("{} repeated ignoring case, e.g. {}", dups.len(), short(dups[0])),
        );
    }
    if strs.len() > MAX_TAGS {
        warn(
            out,
            "tag-count",
            path,
            format!(
                "{} tags (more than {MAX_TAGS}): check it is not a taxonomy dump",
                strs.len()
            ),
        );
    }
}

/// Text a fixture's redaction rewrote ("Xxxxx xx", "字字字"): letters are only x, X and 字, so different words
/// can look identical and duplicates mean nothing.
fn looks_redacted(s: &str) -> bool {
    // Mostly redaction letters: the redactor keeps a few common characters (人, 小, …) as they are.
    let letters: Vec<char> = s.chars().filter(|c| c.is_alphabetic()).collect();
    let marks = letters.iter().filter(|c| matches!(c, 'x' | 'X' | '字')).count();
    !letters.is_empty() && marks * 10 >= letters.len() * 6
}

fn string(path: &str, s: &str, kind: StringKind, out: &mut Vec<Finding>) {
    if kind != StringKind::Html && s.trim() != s && !s.is_empty() {
        err(
            out,
            "text-untrimmed",
            path,
            format!("leading or trailing whitespace in {}", short(s)),
        );
    }
    if kind != StringKind::Html {
        // A named one from the common set, or any numeric one (&#8217;, &#x27;) that decodes.
        let entity = ["&amp;", "&lt;", "&gt;", "&quot;", "&nbsp;", "&apos;"]
            .iter()
            .map(|e| e.to_string())
            .find(|e| s.contains(e.as_str()))
            .or_else(|| numeric_entity(s));
        if let Some(e) = entity {
            err(
                out,
                "text-entity",
                path,
                format!("HTML entity {e} left in {}", short(s)),
            );
        }
    }
    if kind == StringKind::Url && !(s.starts_with("https://") || s.starts_with("http://")) {
        err(
            out,
            "url-relative",
            path,
            format!("not an absolute http(s) URL: {}", short(s)),
        );
    }
}

fn numeric_entity(s: &str) -> Option<String> {
    s.match_indices("&#").find_map(|(i, _)| {
        let rest = &s[i + 2..];
        let end = rest.find(';').filter(|&e| (1..=8).contains(&e))?;
        let body = &rest[..end];
        let ok = match body.strip_prefix(['x', 'X']) {
            Some(hex) => !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()),
            None => body.chars().all(|c| c.is_ascii_digit()),
        };
        ok.then(|| format!("&#{body};"))
    })
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn title(path: &str, field: &str, s: &str, page: &PageFacts, out: &mut Vec<Finding>) {
    let t = norm(s);
    for name in &page.site_names {
        let n = norm(name);
        if n.len() < 2 || looks_redacted(name) {
            continue;
        }
        if t == n && field != "name" {
            err(
                out,
                "title-site",
                path,
                format!("the title is only the site's name {}", short(name)),
            );
            return;
        }
        // "Headline | WSJ", "Headline - wikiHow": the site's name, alone, after a separator at the end.
        for sep in SITE_SEPARATORS {
            if let Some((head, tail)) = s.rsplit_once(sep) {
                let tl = norm(tail);
                if !head.trim().is_empty() && (tl == n || tl == format!("the{n}")) {
                    err(
                        out,
                        "title-site",
                        path,
                        format!("the title ends with the site's name: {}", short(s)),
                    );
                    return;
                }
            }
        }
    }
}

fn is_person(full_name: &str) -> bool {
    full_name.ends_with(".Person") || full_name.ends_with(".Author")
}

fn person(path: &str, v: &serde_json::Value, page: &PageFacts, out: &mut Vec<Finding>) {
    let Some(name) = v.get("name").and_then(|n| n.as_str()) else {
        return;
    };
    let lower = name.to_lowercase();
    let junk = lower.starts_with("by ")
        || lower.contains("http")
        || name.contains('@') && name.contains('.')
        || name.chars().count() > 80;
    if junk {
        err(
            out,
            "person-junk",
            &format!("{path}.name"),
            format!("not a clean name: {}", short(name)),
        );
    } else if page.site_names.iter().any(|s| norm(s) == norm(name)) && !name.is_empty() {
        warn(
            out,
            "person-junk",
            &format!("{path}.name"),
            format!("the site's name as an author: {}", short(name)),
        );
    }
}

/// Times that say when something was published or changed, so cannot be later than the page.
const PUBLICATION_TIMES: &[&str] = &[
    "published_at",
    "updated_at",
    "created_at",
    "modified_at",
    "uploaded_at",
    "posted_at",
    "as_of",
];

fn time(path: &str, field: &str, s: &str, page: &PageFacts, out: &mut Vec<Finding>) {
    let Ok(t) = DateTime::parse_from_rfc3339(s) else { return };
    let t = t.with_timezone(&Utc);
    if PUBLICATION_TIMES.contains(&field) && t > page.fetch_time + chrono::Duration::days(1) {
        err(
            out,
            "time-future",
            path,
            format!(
                "{s} is after the page was fetched ({})",
                page.fetch_time.format("%Y-%m-%d")
            ),
        );
    }
    if t < Utc.with_ymd_and_hms(1990, 1, 1, 0, 0, 0).unwrap() {
        warn(out, "time-ancient", path, format!("{s} is before 1990"));
    }
}

fn price(path: &str, s: &str, out: &mut Vec<Finding>) {
    if let Ok(n) = s.parse::<f64>() {
        if n < 0.0 {
            err(out, "price-negative", path, format!("price {s}"));
        }
    }
}

fn list_items(path: &str, v: &serde_json::Value, _type: &str, out: &mut Vec<Finding>) {
    let Some(arr) = v.as_array() else { return };
    let titled = |x: &serde_json::Value| -> Option<String> {
        ["title", "name", "text", "headline"].iter().find_map(|k| {
            x.get(*k)
                .and_then(|t| t.as_str())
                .filter(|t| !t.trim().is_empty())
                .map(str::to_string)
        })
    };
    // Only lists of titled things (results, posts, products); name/value pairs (specs, figures) are not.
    if arr.len() < 2 || !arr.iter().any(|x| titled(x).is_some()) || arr.iter().any(|x| x.get("value").is_some()) {
        return;
    }
    let untitled = arr.iter().filter(|x| x.is_object() && titled(x).is_none()).count();
    if untitled > 0 {
        warn(
            out,
            "item-untitled",
            path,
            format!("{untitled} of {} items have no title or name", arr.len()),
        );
    }
    let urls: Vec<&str> = arr
        .iter()
        .filter_map(|x| x.get("url").and_then(|u| u.as_str()).filter(|u| !u.is_empty()))
        .collect();
    let mut seen = std::collections::HashSet::new();
    let dups = urls.iter().filter(|u| !seen.insert(**u)).count();
    // Most items sharing a URL means it is the page's (how-to steps), not each item's link.
    if dups > 0 && dups * 2 < urls.len() {
        warn(
            out,
            "item-duplicate",
            path,
            format!("{dups} items repeat an earlier item's URL"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_codes_are_codes_and_topics_are_not() {
        // Arrange
        let codes = [
            "LINK|i13-WP-WSJ-0004031401",
            "l1:politics",
            "n/elx",
            "135740207",
            "c314",
            "wc_401_600",
            "LINK:EN",
        ];
        let topics = [
            "Elections",
            "Middle East",
            "AI",
            "C++",
            "iOS 18",
            "2024",
            "Donald Trump",
            "N/A",
            "Politics/International Relations",
            "G20",
            "F35",
            "x86",
            "COVID-19",
            "AT&T",
            "Web 3.0",
            "人工智能",
        ];

        // Act
        let code_hits: Vec<bool> = codes.iter().map(|c| is_code(c)).collect();
        let topic_hits: Vec<&str> = topics.iter().copied().filter(|t| is_code(t)).collect();

        // Assert
        assert!(code_hits.iter().all(|&h| h), "{code_hits:?}");
        assert!(topic_hits.is_empty(), "{topic_hits:?}");
    }
}
