//! Fixture redaction (decisions.md U19). Committed fixtures must not carry publisher content, but
//! recipes must keep working on them. So structure is kept (tags, id, class, href, src, datetime, lang,
//! meta keys, JSON-LD keys and @type) and text is replaced character by character:
//!   Latin letters -> x / X (case kept), other letters -> x, CJK -> 字,
//!   digits, punctuation, whitespace and URL-shaped tokens are kept,
//!   date vocabulary (month/weekday names, am/pm, zone abbreviations, relative-date words and the CJK date
//!   characters) is kept so date parsing still works.
//! Non-JSON-LD scripts, styles and raw-text elements (noscript, iframe, …) are emptied; comments are
//! dropped. The output is deterministic and idempotent.

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use serde_json::Value;

const KEEP_WORDS: &[&str] = &[
    // shop and search UI labels recipes parse (decisions.md U32): "In Stock", "4.6 out of 5 stars",
    // "Currently unavailable", "163,000 results", "934 ratings"
    "in",
    "out",
    "of",
    "stock",
    "currently",
    "unavailable",
    "available",
    "rating",
    "ratings",
    "result",
    "results",
    // US shop, search and social labels (decisions.md U35): "Average customer rating is 4.6 out of 5 stars
    // with 4974 reviews", "Add to cart", "Sponsored", "Answer (1 of 422)", "Updated 8y", "Verified",
    // "View this user's verifications", "View profile"
    "average",
    "customer",
    "is",
    "with",
    "review",
    "reviews",
    "add",
    "to",
    "cart",
    "sponsored",
    "answer",
    "answers",
    "updated",
    "verified",
    "verifications",
    "profile",
    // quote-page row labels (decisions.md U37): "Prev Close", "52 Week High", "Market Cap", "PE Ratio (TTM)",
    // "Forward Dividend & Yield", "Expense Ratio (net)", "Net Assets", "Circulating supply", "After Hours"
    "previous",
    "prev",
    "close",
    "open",
    "high",
    "low",
    "volume",
    "market",
    "cap",
    "pe",
    "eps",
    "ttm",
    "forward",
    "dividend",
    "yield",
    "expense",
    "ratio",
    "net",
    "assets",
    "circulating",
    "supply",
    "after",
    "hours",
    "last",
    "at",
    "as",
    "by",
    "overnight",
    "pre",
    "p",
    "e",
    "cryptocurrency",
    // social metric labels in aria-labels ("1574 reposts, 657 bookmarks")
    "repost",
    "reposts",
    "retweet",
    "retweets",
    "bookmark",
    "bookmarks",
    "quote",
    "quotes",
    // months
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
    "jan",
    "feb",
    "mar",
    "apr",
    "jun",
    "jul",
    "aug",
    "sep",
    "sept",
    "oct",
    "nov",
    "dec",
    // weekdays
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "mon",
    "tue",
    "tues",
    "wed",
    "thu",
    "thur",
    "thurs",
    "fri",
    "sat",
    "sun",
    // clock / zones
    "am",
    "pm",
    "a",
    "m",
    "utc",
    "gmt",
    "et",
    "est",
    "edt",
    "ct",
    "cst",
    "cdt",
    "mt",
    "mst",
    "mdt",
    "pt",
    "pst",
    "pdt",
    "bst",
    "cet",
    "cest",
    "ist",
    "jst",
    "kst",
    "hkt",
    "sgt",
    "aest",
    "aedt",
    "z",
    "t",
    // relative dates and ordinals
    "ago",
    "just",
    "now",
    "yesterday",
    "today",
    "second",
    "seconds",
    "sec",
    "secs",
    "minute",
    "minutes",
    "min",
    "mins",
    "hour",
    "hours",
    "hr",
    "hrs",
    "day",
    "days",
    "week",
    "weeks",
    "month",
    "months",
    "year",
    "years",
    "st",
    "nd",
    "rd",
    "th",
    // counter labels (UI vocabulary recipes match on, not publisher prose)
    "comment",
    "comments",
    "point",
    "points",
    "vote",
    "votes",
    "like",
    "likes",
    "view",
    "views",
    "reply",
    "replies",
    "star",
    "stars",
    "fork",
    "forks",
    "share",
    "shares",
    "discuss",
];

/// Suffixes kept when they directly follow a digit: magnitudes (K, M, B) and compact ages (s, m, h, d, w,
/// mo, y, yr).
const NUMBER_UNITS: &[&str] = &["k", "m", "b", "s", "h", "d", "w", "mo", "y", "yr", "min", "hr"];

/// CJK characters kept: date vocabulary (年月日 时分秒 前 小时 分钟 刚刚 昨天 今天 星期X 上午 下午 周),
/// counter units (评论 赞 阅读 条 次 人 万 亿) and quote-row labels (今开 昨收 最高 最低 成交量 成交额 总市值
/// 市盈率 换手 振幅 单位净值 累计净值 估算 日增长率 股息率 每股收益 规模 元 货币单位 交易中 已收盘 休市 盘前/盘后)
/// that recipes match on.
const KEEP_CJK: &str = "年月日时分秒点前天周昨今刚小钟星期一二三四五六七上下午评论赞阅读条次人万亿\
    开收高低停换手成交量额市值盈净比涨跌幅总流通率单位累计估算增长振股息每益最动静规模元货币易中已盘休后";

const VOID_TAGS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr",
];

/// Attributes whose values are structure, not prose.
const KEEP_ATTRS: &[&str] = &[
    "href",
    "src",
    "srcset",
    "datetime",
    "class",
    "id",
    "lang",
    "rel",
    "type",
    "name",
    "property",
    "itemprop",
    "itemscope",
    "itemtype",
    "itemid",
    "http-equiv",
    "charset",
    "width",
    "height",
    "colspan",
    "rowspan",
    "role",
    "for",
    "target",
    "media",
    "sizes",
    "loading",
    "dir",
    "hidden",
    "style",
    "action",
    "method",
    "async",
    "defer",
    "crossorigin",
    "integrity",
    "tabindex",
    "aria-hidden",
    "slot",
    "aria-labelledby",
    "aria-describedby",
    "start",
    "span",
    "headers",
    "scope",
    "poster",
    "data",
];

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF | 0x20000..=0x2FA1F)
}

/// Characters a URL can contain (RFC 3986 plus `%`, and `{}` of URL templates such as
/// "?s={search_term_string}"); anything else (whitespace, CJK, an em dash) ends one.
fn url_char(c: char) -> bool {
    c.is_ascii_graphic() && !matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '|')
}

/// The whole string is one URL, with nothing after it. A URL followed by prose ("https://e.com The full
/// story") is not, and goes through `redact_text`, which keeps the URL and redacts the rest.
fn url_shaped(s: &str) -> bool {
    let t = s.trim();
    (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www.") || t.starts_with('/'))
        && t.chars().all(url_char)
}

/// The whole string is a numeric date or date-time ("2026-10-16", "2026/10/16 09:30", ISO 8601 with a
/// zone). A date followed by prose ("2026/10/16 北京电 ...") is not.
fn date_shaped(s: &str) -> bool {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"^\d{4}[-/]\d{1,2}[-/]\d{1,2}(?:[T ]\d{1,2}:\d{2}(?::\d{2}(?:\.\d+)?)?)?(?:Z|[+-]\d{2}:?\d{2})?$",
        )
        .expect("static regex")
    })
    .is_match(s.trim())
}

/// Redacts prose carried in a URL's query values or fragment (share links put the headline in
/// `?text=...`, mail links in `?subject=...&body=...`). A value is prose when it decodes to something with
/// whitespace or CJK; ids, tokens and nested URLs are kept, so recipes can still read them.
pub fn redact_url(u: &str) -> String {
    let (before_frag, frag) = match u.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (u, None),
    };
    let (base, query) = match before_frag.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (before_frag, None),
    };
    let mut out = base.to_string();
    if let Some(q) = query {
        out.push('?');
        let pairs: Vec<String> = q
            .split('&')
            .map(|pair| match pair.split_once('=') {
                Some((k, v)) => format!("{k}={}", redact_component(v)),
                None => redact_component(pair),
            })
            .collect();
        out.push_str(&pairs.join("&"));
    }
    if let Some(f) = frag {
        out.push('#');
        out.push_str(&redact_component(f));
    }
    out
}

fn redact_component(v: &str) -> String {
    let decoded = percent_decode(&v.replace('+', " "));
    if is_prose(&decoded) {
        percent_encode(&redact_text(decoded.trim()))
    } else {
        v.to_string()
    }
}

/// Text rather than a token: any CJK, or at least two whitespace-separated words of letters ("Senate passes
/// bill"; not base64 whose `+` decoded to a space, not "Space Mono:wght@400").
fn is_prose(s: &str) -> bool {
    s.chars().any(is_cjk)
        || s.split_whitespace()
            .filter(|w| w.chars().count() >= 2 && w.chars().all(char::is_alphabetic))
            .count()
            >= 2
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A short identifier-like value (enum, language code, MIME/OG type) that is not prose.
fn token_shaped(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() <= 40 && !t.contains(char::is_whitespace) && {
        let first = t.chars().next().unwrap_or(' ');
        first.is_ascii_uppercase() && t.chars().all(|c| c.is_ascii_alphanumeric())
            || t.chars().all(|c| {
                c.is_ascii_lowercase() || c == '-' || c == '_' || c == ':' || c == '/' || c == '.' || c.is_ascii_digit()
            }) && t.len() <= 16
            || is_language_tag(t)
    }
}

fn is_language_tag(t: &str) -> bool {
    let mut parts = t.split(['-', '_']);
    let primary = parts.next().unwrap_or("");
    (2..=3).contains(&primary.len())
        && primary.chars().all(|c| c.is_ascii_lowercase())
        && parts.all(|p| (2..=4).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Redact one run of prose.
pub fn redact_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let lower = word.to_lowercase();
        // A unit glued to a number is part of the number: "1.7K", "9.5M", "3d", "2mo", "11y".
        let unit = out.ends_with(|c: char| c.is_ascii_digit()) && NUMBER_UNITS.contains(&lower.as_str());
        if unit || KEEP_WORDS.contains(&lower.as_str()) {
            out.push_str(word);
        } else {
            for c in word.chars() {
                out.push(if c.is_uppercase() { 'X' } else { 'x' });
            }
        }
        word.clear();
    };
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        // A URL token: kept (minus prose in its query) up to the first character a URL cannot contain, so
        // CJK prose glued to a link ("…/a今天总统宣布…") is still redacted.
        if word.is_empty()
            && (s[i..].starts_with("http://") || s[i..].starts_with("https://") || s[i..].starts_with("www."))
        {
            let end = s[i..].find(|c: char| !url_char(c)).map(|n| i + n).unwrap_or(s.len());
            out.push_str(&redact_url(&s[i..end]));
            while chars.peek().is_some_and(|(j, _)| *j < end) {
                chars.next();
            }
            continue;
        }
        if is_cjk(c) {
            flush(&mut word, &mut out);
            out.push(if KEEP_CJK.contains(c) { c } else { '字' });
        } else if c.is_alphabetic() {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

fn redact_json(v: &mut Value, key: Option<&str>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map.iter_mut() {
                redact_json(child, Some(k));
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_json(item, key);
            }
        }
        Value::String(s) => {
            // Only structural JSON-LD keywords are kept as-is; "@value" holds the text itself.
            let structural = matches!(key, Some("@type" | "@context" | "@language" | "@vocab"))
                || (key == Some("@id") && (s.starts_with('#') || s.starts_with("_:") || token_shaped(s)));
            if structural || date_shaped(s) || token_shaped(s) {
                return;
            }
            *s = if url_shaped(s) { redact_url(s) } else { redact_text(s) };
        }
        _ => {}
    }
}

/// Redact a JSON-LD block; unparseable blocks become empty.
pub fn redact_jsonld(raw: &str) -> String {
    match serde_json::from_str::<Value>(raw.trim()) {
        Ok(mut v) => {
            redact_json(&mut v, None);
            serde_json::to_string(&v).unwrap_or_default()
        }
        Err(_) => String::new(),
    }
}

/// Kept attributes that hold a URL, whose query may still carry prose.
const URL_ATTRS: &[&str] = &[
    "href",
    "src",
    "action",
    "poster",
    "data",
    "cite",
    "formaction",
    "background",
];

fn redact_attr(el_name: &str, name: &str, value: &str, el: &ElementRef<'_>) -> String {
    let n = name.to_ascii_lowercase();
    if URL_ATTRS.contains(&n.as_str()) {
        return redact_url(value);
    }
    if KEEP_ATTRS.contains(&n.as_str()) {
        return value.to_string();
    }
    if n == "content" && el_name == "meta" {
        let key = el
            .value()
            .attr("property")
            .or_else(|| el.value().attr("name"))
            .or_else(|| el.value().attr("itemprop"))
            .unwrap_or("")
            .to_ascii_lowercase();
        let structural = key.ends_with(":type")
            || key.ends_with("_time")
            || key.contains("locale")
            || key == "viewport"
            || key == "robots"
            || key.starts_with("twitter:card");
        if structural || date_shaped(value) || token_shaped(value) {
            return value.to_string();
        }
        if url_shaped(value) {
            return redact_url(value);
        }
        return redact_text(value);
    }
    if n.starts_with("data-") {
        // A space-free value is usually an id or flag, but CJK prose has no spaces.
        let token = !value.contains(char::is_whitespace) && !value.chars().any(is_cjk);
        if url_shaped(value) {
            return redact_url(value);
        }
        if date_shaped(value) || token {
            return value.to_string();
        }
        return redact_text(value);
    }
    if url_shaped(value) {
        return redact_url(value);
    }
    if date_shaped(value) {
        return value.to_string();
    }
    redact_text(value)
}

fn escape(s: &str, attr: bool, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' if !attr => out.push_str("&lt;"),
            '>' if !attr => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\u{00A0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
}

fn serialize(node: NodeRef<'_, Node>, out: &mut String) {
    match node.value() {
        Node::Doctype(d) => {
            out.push_str("<!DOCTYPE ");
            out.push_str(&d.name);
            out.push_str(">\n");
        }
        Node::Text(t) => escape(&redact_text(t), false, out),
        Node::Element(e) => {
            let el = ElementRef::wrap(node).expect("element");
            let name = e.name();
            out.push('<');
            out.push_str(name);
            for (k, v) in e.attrs() {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                escape(&redact_attr(name, k, v, &el), true, out);
                out.push('"');
            }
            out.push('>');
            if VOID_TAGS.contains(&name) {
                return;
            }
            match name {
                "script" => {
                    let ty = e.attr("type").unwrap_or("").trim().to_ascii_lowercase();
                    if ty == "application/ld+json" {
                        let raw: String = el.text().collect();
                        out.push_str(&redact_jsonld(&raw).replace("</", "<\\/"));
                    }
                }
                // Raw-text elements: their content is not DOM (scripting-enabled parsing) and never
                // needed for extraction, so it is dropped like inline scripts.
                "style" | "noscript" | "iframe" | "noembed" | "noframes" | "xmp" => {}
                _ => {
                    for child in node.children() {
                        serialize(child, out);
                    }
                }
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        }
        Node::Document | Node::Fragment => {
            for child in node.children() {
                serialize(child, out);
            }
        }
        _ => {}
    }
}

/// Redact a whole HTML document.
pub fn redact_html(html: &str) -> String {
    let mut doc = Html::parse_document(html);
    rex_dom::prune_deep(&mut doc, rex_dom::MAX_DOM_DEPTH);
    let mut out = String::with_capacity(html.len());
    serialize(doc.tree.root(), &mut out);
    out
}

/// First word that redaction would still change (the lint is "redaction is a no-op").
fn first_change(original: &str, redacted: &str) -> Option<String> {
    if original == redacted {
        return None;
    }
    let o: Vec<&str> = original.split_whitespace().collect();
    let r: Vec<&str> = redacted.split_whitespace().collect();
    let word = o
        .iter()
        .zip(r.iter())
        .find(|(a, b)| a != b)
        .map(|(a, _)| *a)
        .unwrap_or(original);
    Some(word.chars().take(60).collect())
}

/// Find prose that was not redacted (FR-RC-5). A fixture is clean iff redacting it again changes
/// nothing, so the lint and the redactor can never disagree. Empty means clean.
pub fn lint_html(html: &str) -> Vec<String> {
    let mut doc = Html::parse_document(html);
    rex_dom::prune_deep(&mut doc, rex_dom::MAX_DOM_DEPTH);
    let mut out = Vec::new();
    for node in doc.tree.root().descendants() {
        if out.len() >= 20 {
            break;
        }
        match node.value() {
            Node::Text(t) => {
                let parent = node.parent().and_then(ElementRef::wrap);
                let parent_name = parent.map(|p| p.value().name().to_string()).unwrap_or_default();
                if parent_name == "script" {
                    let is_jsonld = parent
                        .and_then(|p| p.value().attr("type"))
                        .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/ld+json"));
                    if !is_jsonld {
                        if !t.trim().is_empty() {
                            out.push("script: inline script content must be dropped".into());
                        }
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(t.trim()) else {
                        out.push("json-ld: unparseable block".into());
                        continue;
                    };
                    let normalized = serde_json::to_string(&v).unwrap_or_default();
                    if let Some(w) = first_change(&normalized, &redact_jsonld(&normalized)) {
                        out.push(format!("json-ld: unredacted `{w}`"));
                    }
                } else if ["noscript", "iframe", "noembed", "noframes", "xmp"].contains(&parent_name.as_str()) {
                    if !t.trim().is_empty() {
                        out.push(format!("<{parent_name}>: raw-text content must be dropped"));
                    }
                } else if parent_name == "style" {
                    if !t.trim().is_empty() {
                        out.push("<style>: content must be dropped".into());
                    }
                } else if let Some(w) = first_change(t, &redact_text(t)) {
                    out.push(format!("<{parent_name}> text: unredacted `{w}`"));
                }
            }
            Node::Comment(c) if !c.trim().is_empty() => {
                out.push("comment: comments must be dropped".into());
            }
            Node::Element(e) => {
                let el = ElementRef::wrap(node).expect("element");
                for (k, v) in e.attrs() {
                    if let Some(w) = first_change(v, &redact_attr(e.name(), k, v, &el)) {
                        out.push(format!("<{}> @{k}: unredacted `{w}`", e.name()));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_glued_to_or_prefixed_by_urls_and_dates_is_redacted() {
        // Arrange: the leak classes found in the 2026-10-16 review
        let html = r#"<html><head><meta name="description" content="2026-10-16 The president announced a secret plan">
<script type="application/ld+json">{"@type":"NewsArticle","articleBody":"2026/10/16 北京电 总统宣布了改革","name":{"@value":"Secret headline value"},"url":"https://e.com/a"}</script></head>
<body><p>来源：https://example.com/a今天总统宣布了改革</p><img alt="www.example.com The senator was photographed" src="/p.jpg">
<a href="https://twitter.com/intent/tweet?text=Senate+passes+secret+bill&amp;via=news">share</a>
<a href="https://www.youtube.com/watch?v=dQw4w9WgXcQ">ok</a><a href="https://e.com/x?q=%E6%84%8F%E5%BC%8F%E5%92%96%E5%95%A1">s</a></body></html>"#;

        // Act
        let r = redact_html(html);

        // Assert
        for leaked in ["president", "总统", "Secret headline", "senator", "Senate", "%E6%84%8F"] {
            assert!(!r.contains(leaked), "`{leaked}` survived:\n{r}");
        }
        assert!(r.contains("https://example.com/a") && r.contains("watch?v=dQw4w9WgXcQ") && r.contains("via=news"));
        assert!(r.contains(r#""url":"https://e.com/a""#) && r.contains("2026-10-16 Xxx"));
        assert!(lint_html(&r).is_empty(), "{:?}", lint_html(&r));
    }

    #[test]
    fn lint_flags_comments_and_style_text() {
        // Arrange
        let html = "<html><head><style>.a{content:\"secret prose\"}</style></head><body><!-- secret editorial note --><p>x</p></body></html>";

        // Act
        let v = lint_html(html);

        // Assert
        assert!(
            v.iter().any(|m| m.contains("comment")) && v.iter().any(|m| m.contains("<style>")),
            "{v:?}"
        );
    }

    #[test]
    fn text_is_replaced_but_dates_digits_and_urls_survive() {
        // Arrange
        let s = "Fed Holds Rates — Oct. 11, 2026 9:30 am ET, see https://example.com/a?b=1 for 3 Details";

        // Act
        let r = redact_text(s);

        // Assert
        assert_eq!(
            r,
            "Xxx Xxxxx Xxxxx — Oct. 11, 2026 9:30 am ET, xxx https://example.com/a?b=1 xxx 3 Xxxxxxx"
        );
    }

    #[test]
    fn cjk_is_replaced_except_date_characters() {
        // Arrange / Act
        let r = redact_text("用声明式配方 2026年10月8日 3小时前");

        // Assert
        assert_eq!(r, "字字字字字字 2026年10月8日 3小时前");
    }

    #[test]
    fn html_keeps_structure_drops_scripts_and_redacts_jsonld_strings() {
        // Arrange
        let html = r#"<!doctype html><html lang="en"><head><title>Secret Title</title>
<meta property="og:title" content="Secret Title"><meta property="og:type" content="article">
<meta property="article:published_time" content="2026-10-11T09:30:00Z">
<script>trackUser("secret")</script>
<script type="application/ld+json">{"@type":"NewsArticle","headline":"Secret Title","url":"https://e.com/a","datePublished":"2026-10-11","inLanguage":"en-US"}</script>
<style>.a{color:red}</style></head>
<body><article class="story" id="main"><h1>Secret Title</h1><img src="/x.png" alt="A photo"><a href="/next" title="Read more">Next story</a><time datetime="2026-10-11">Oct 11</time></article><!-- comment --></body></html>"#;

        // Act
        let r = redact_html(html);

        // Assert
        assert!(
            r.contains(r#"<article class="story" id="main"><h1>Xxxxxx Xxxxx</h1>"#),
            "{r}"
        );
        assert!(r.contains(r#"alt="A xxxxx""#) && r.contains(r#"src="/x.png""#));
        assert!(r.contains(r#"content="article""#) && r.contains(r#"content="2026-10-11T09:30:00Z""#));
        assert!(r.contains(r#"content="Xxxxxx Xxxxx""#));
        assert!(
            r.contains(r#""headline":"Xxxxxx Xxxxx""#)
                && r.contains(r#""url":"https://e.com/a""#)
                && r.contains(r#""inLanguage":"en-US""#)
        );
        assert!(!r.contains("trackUser") && !r.contains("color:red") && !r.contains("comment"));
        assert!(r.contains(r#"<time datetime="2026-10-11">Oct 11</time>"#));
        assert!(lint_html(&r).is_empty(), "{:?}", lint_html(&r));
    }

    #[test]
    fn lint_flags_unredacted_prose() {
        // Arrange
        let html = "<p>Federal Reserve officials held rates</p><img alt=\"A photo of the chair\">";

        // Act
        let v = lint_html(html);

        // Assert
        assert!(v.iter().any(|m| m.contains("Federal")));
        assert!(v.iter().any(|m| m.contains("@alt")));
    }

    #[test]
    fn units_glued_to_numbers_survive() {
        // Arrange
        let s = "1.7K likes, 63K, 3d ago, 2mo, 11y Kd";

        // Act
        let r = redact_text(s);

        // Assert
        assert_eq!(r, "1.7K likes, 63K, 3d ago, 2mo, 11y Xx");
    }

    #[test]
    fn cjk_data_attributes_are_redacted_but_tokens_survive() {
        // Arrange
        let html = r#"<h1 data-title="再访" data-v-9f85ca64="" data-id="BV149WF6zEw2">再访</h1>"#;

        // Act
        let out = redact_html(html);

        // Assert
        assert!(!out.contains("再访"), "{out}");
        assert!(out.contains(r#"data-id="BV149WF6zEw2""#), "{out}");
        assert!(lint_html(html).iter().any(|m| m.contains("@data-title")));
    }

    #[test]
    fn redaction_is_deterministic_and_idempotent_on_structure() {
        // Arrange
        let html = "<html><body><p class=x>Hello World 世界</p></body></html>";

        // Act
        let once = redact_html(html);
        let twice = redact_html(&once);

        // Assert
        assert_eq!(once, twice);
    }
}
