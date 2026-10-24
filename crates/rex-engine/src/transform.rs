//! Closed transform set (decisions.md U8). Transforms operate on the list of items a candidate
//! produced; string transforms map item by item, list transforms see the whole list.

use regex::{Regex, RegexBuilder};
use rex_dom::text::{normalize_string, normalize_text};
use rex_dom::{Budget, BudgetExceeded};
use rex_proto::extract as pb;
use serde_json::Value as Json;

use crate::values::{self, DatetimeSpec, UrlSpec};

/// One value flowing through a pipe.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A string. `text_ready` marks block text that is already TEXT-normalized (keeps `pre`).
    Str { s: String, text_ready: bool },
    /// A structured JSON-LD / microdata value (object).
    Json(Json),
}

impl Item {
    pub fn str(s: impl Into<String>) -> Item {
        Item::Str {
            s: s.into(),
            text_ready: false,
        }
    }

    pub fn text(s: impl Into<String>) -> Item {
        Item::Str {
            s: s.into(),
            text_ready: true,
        }
    }

    pub fn from_json(v: Json) -> Option<Item> {
        match v {
            Json::Null => None,
            Json::String(s) => Some(Item::str(s)),
            // serde_json prints floats like 1e21 in exponent form, which number parsing would read as 1.
            Json::Number(n) => Some(Item::str(match n.as_f64() {
                Some(f) if n.is_f64() => format!("{f}"),
                _ => n.to_string(),
            })),
            Json::Bool(b) => Some(Item::str(b.to_string())),
            Json::Array(_) => None,
            obj @ Json::Object(_) => Some(Item::Json(obj)),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Item::Str { s, .. } => Some(s),
            Item::Json(_) => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Item::Str { s, .. } => s.trim().is_empty(),
            Item::Json(v) => v.as_object().is_none_or(|o| o.is_empty()),
        }
    }

    /// Short preview for `explain`.
    pub fn preview(&self) -> String {
        let s = match self {
            Item::Str { s, .. } => s.clone(),
            Item::Json(v) => v.to_string(),
        };
        let mut out: String = s.chars().take(120).collect();
        if s.chars().count() > 120 {
            out.push('…');
        }
        out
    }
}

#[derive(Clone, Debug)]
pub enum Transform {
    Trim,
    CollapseWs,
    Case {
        upper: bool,
    },
    Strip {
        prefix: String,
        suffix: String,
    },
    Truncate(usize),
    Regex {
        re: Regex,
        group: usize,
    },
    RegexReplace {
        re: Regex,
        with: String,
    },
    Split(Regex),
    Join(String),
    Slice {
        start: i32,
        end: i32,
    },
    Unique,
    Filter {
        re: Regex,
        invert: bool,
    },
    HtmlToText,
    JsonPath(String),
    Datetime(DatetimeSpec),
    Number {
        locale: String,
        cjk_units: bool,
    },
    Url(UrlSpec),
    Map {
        entries: Vec<(String, String)>,
        drop_unmapped: bool,
    },
    Duration,
    Decode(pb::DecodeKind),
}

/// Compile one proto transform. `Default` is handled by the rule, so it is reported separately.
pub enum Compiled {
    Op(Transform),
    Default(String),
}

pub fn compile_regex(pattern: &str, max_bytes: usize) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .size_limit(max_bytes)
        .build()
        .map_err(|e| format!("regex `{pattern}` is invalid or outside the RE2 subset: {e}"))
}

pub fn compile(t: &pb::Transform, max_regex_bytes: usize) -> Result<Compiled, String> {
    use pb::transform::Op;
    let op = t.op.as_ref().ok_or("empty transform (no operation set)")?;
    let re = |p: &str| compile_regex(p, max_regex_bytes);
    Ok(Compiled::Op(match op {
        Op::Trim(_) => Transform::Trim,
        Op::CollapseWs(_) => Transform::CollapseWs,
        Op::ChangeCase(c) => Transform::Case { upper: c.upper },
        Op::Strip(s) => Transform::Strip {
            prefix: s.prefix.clone(),
            suffix: s.suffix.clone(),
        },
        Op::Truncate(t) => Transform::Truncate(t.chars as usize),
        Op::Regex(r) => {
            let compiled = re(&r.pattern)?;
            if r.group as usize >= compiled.captures_len() {
                return Err(format!("regex `{}` has no capture group {}", r.pattern, r.group));
            }
            Transform::Regex {
                re: compiled,
                group: r.group as usize,
            }
        }
        Op::RegexReplace(r) => Transform::RegexReplace {
            re: re(&r.pattern)?,
            with: r.with.clone(),
        },
        Op::Split(s) => Transform::Split(re(&s.pattern)?),
        Op::Join(j) => Transform::Join(j.separator.clone()),
        Op::Slice(s) => Transform::Slice {
            start: s.start,
            end: s.end,
        },
        Op::Unique(_) => Transform::Unique,
        Op::Filter(f) => Transform::Filter {
            re: re(&f.pattern)?,
            invert: f.invert,
        },
        Op::HtmlToText(_) => Transform::HtmlToText,
        Op::Json(j) => {
            rex_dom::jsonld::validate_path(&j.path)?;
            Transform::JsonPath(j.path.clone())
        }
        Op::Datetime(d) => {
            let tz = if d.tz.is_empty() {
                None
            } else {
                Some(
                    d.tz.parse::<chrono_tz::Tz>()
                        .map_err(|_| format!("time zone `{}` is unknown or not compiled in (see CHRONO_TZ_TIMEZONE_FILTER in .cargo/config.toml)", d.tz))?,
                )
            };
            if !matches!(d.locale.as_str(), "" | "en" | "zh") {
                return Err(format!("unsupported datetime locale `{}` (en, zh)", d.locale));
            }
            Transform::Datetime(DatetimeSpec {
                formats: d.formats.clone(),
                tz,
                locale: d.locale.clone(),
                relative: d.relative,
            })
        }
        Op::Number(n) => Transform::Number {
            locale: n.locale.clone(),
            cjk_units: n.cjk_units,
        },
        Op::Url(u) => Transform::Url(UrlSpec {
            strip_query: u.strip_query.clone(),
            keep_fragment: u.keep_fragment,
        }),
        Op::Map(m) => Transform::Map {
            entries: m
                .entries
                .iter()
                .map(|e| (e.from.trim().to_lowercase(), e.to.clone()))
                .collect(),
            drop_unmapped: m.drop_unmapped,
        },
        Op::Default(d) => return Ok(Compiled::Default(d.value.clone())),
        Op::Duration(_) => Transform::Duration,
        Op::Decode(d) => match pb::DecodeKind::try_from(d.kind) {
            Ok(pb::DecodeKind::Unspecified) | Err(_) => return Err("decode needs a kind".into()),
            Ok(k) => Transform::Decode(k),
        },
    }))
}

/// Context a pipe needs beyond its items.
pub struct PipeCtx<'a> {
    pub base: &'a url::Url,
    pub now: chrono::DateTime<chrono::Utc>,
    pub budget: &'a Budget,
}

/// Why a pipe dropped everything.
pub type PipeResult = Result<Result<Vec<Item>, String>, BudgetExceeded>;

/// Most items any transform may produce (`split` of a hostile string); the field's `max_items` applies later.
pub const MAX_PIPE_ITEMS: usize = 10_000;

/// Fuel for running one transform over `items`: a step per item plus one per 16 bytes of text, so a hostile
/// megabyte string costs in proportion to the work done on it.
fn cost(items: &[Item]) -> u64 {
    let bytes: usize = items
        .iter()
        .map(|i| match i {
            Item::Str { s, .. } => s.len(),
            Item::Json(_) => 64,
        })
        .sum();
    1 + items.len() as u64 + (bytes / 16) as u64
}

pub fn apply(pipe: &[Transform], mut items: Vec<Item>, ctx: &PipeCtx<'_>) -> PipeResult {
    for t in pipe {
        ctx.budget.spend(cost(&items))?;
        items = match apply_one(t, items, ctx) {
            Ok(v) => v,
            Err(reason) => return Ok(Err(reason)),
        };
        items.truncate(MAX_PIPE_ITEMS);
        if items.is_empty() {
            return Ok(Err(format!("transform {} left no value", name(t))));
        }
    }
    Ok(Ok(items))
}

fn name(t: &Transform) -> &'static str {
    match t {
        Transform::Trim => "trim",
        Transform::CollapseWs => "collapse_ws",
        Transform::Case { .. } => "change_case",
        Transform::Strip { .. } => "strip",
        Transform::Truncate(_) => "truncate",
        Transform::Regex { .. } => "regex",
        Transform::RegexReplace { .. } => "regex_replace",
        Transform::Split(_) => "split",
        Transform::Join(_) => "join",
        Transform::Slice { .. } => "slice",
        Transform::Unique => "unique",
        Transform::Filter { .. } => "filter",
        Transform::HtmlToText => "html_to_text",
        Transform::JsonPath(_) => "json",
        Transform::Datetime(_) => "datetime",
        Transform::Number { .. } => "number",
        Transform::Url(_) => "url",
        Transform::Map { .. } => "map",
        Transform::Duration => "duration",
        Transform::Decode(_) => "decode",
    }
}

/// Map each string item; objects are passed through untouched by string-only transforms.
fn map_str(items: Vec<Item>, f: impl Fn(&str) -> Option<String>) -> Vec<Item> {
    items
        .into_iter()
        .filter_map(|it| match it {
            Item::Str { s, .. } => f(&s).map(Item::str),
            obj @ Item::Json(_) => Some(obj),
        })
        .collect()
}

fn apply_one(t: &Transform, items: Vec<Item>, ctx: &PipeCtx<'_>) -> Result<Vec<Item>, String> {
    Ok(match t {
        Transform::Trim => items
            .into_iter()
            .map(|it| match it {
                Item::Str { s, text_ready } => Item::Str {
                    s: s.trim().to_string(),
                    text_ready,
                },
                other => other,
            })
            .collect(),
        Transform::CollapseWs => map_str(items, |s| Some(normalize_string(s))),
        Transform::Case { upper } => map_str(items, |s| {
            Some(if *upper { s.to_uppercase() } else { s.to_lowercase() })
        }),
        Transform::Strip { prefix, suffix } => map_str(items, |s| {
            let s = s.strip_prefix(prefix.as_str()).unwrap_or(s);
            Some(s.strip_suffix(suffix.as_str()).unwrap_or(s).to_string())
        }),
        Transform::Truncate(n) => map_str(items, |s| Some(s.chars().take(*n).collect())),
        Transform::Regex { re, group } => map_str(items, |s| {
            re.captures(s)
                .and_then(|c| c.get(*group))
                .map(|m| m.as_str().to_string())
        }),
        Transform::RegexReplace { re, with } => map_str(items, |s| Some(re.replace_all(s, with.as_str()).into_owned())),
        Transform::Split(re) => items
            .into_iter()
            .flat_map(|it| match it {
                Item::Str { s, .. } => re
                    .split(&s)
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .take(MAX_PIPE_ITEMS)
                    .map(Item::str)
                    .collect::<Vec<_>>(),
                obj => vec![obj],
            })
            .take(MAX_PIPE_ITEMS)
            .collect(),
        Transform::Join(sep) => {
            let parts: Vec<String> = items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect();
            if parts.is_empty() {
                Vec::new()
            } else {
                vec![Item::str(parts.join(sep))]
            }
        }
        Transform::Slice { start, end } => slice(items, *start, *end),
        Transform::Unique => {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            items
                .into_iter()
                .filter(|it| {
                    let key = match it {
                        Item::Str { s, .. } => s.clone(),
                        Item::Json(v) => v.to_string(),
                    };
                    seen.insert(key)
                })
                .collect()
        }
        Transform::Filter { re, invert } => items
            .into_iter()
            .filter(|it| it.as_str().is_some_and(|s| re.is_match(s) != *invert))
            .collect(),
        Transform::HtmlToText => items
            .into_iter()
            .filter_map(|it| match it {
                Item::Str { s, .. } => {
                    let frag = scraper::Html::parse_fragment(&s);
                    rex_dom::text::block_text(frag.root_element(), ctx.budget)
                        .ok()
                        .map(Item::text)
                }
                obj => Some(obj),
            })
            .collect(),
        Transform::JsonPath(path) => {
            let mut out = Vec::new();
            for it in items {
                let parsed: Json = match &it {
                    Item::Str { s, .. } => match serde_json::from_str(s) {
                        Ok(v) => v,
                        Err(_) => continue,
                    },
                    Item::Json(v) => v.clone(),
                };
                let values = rex_dom::jsonld::JsonLdIndex::default().eval_path(&parsed, path)?;
                out.extend(values.into_iter().filter_map(Item::from_json));
            }
            out
        }
        Transform::Datetime(spec) => map_str(items, |s| {
            let parsed = values::parse_datetime(s, spec, ctx.now)?;
            Some(
                parsed
                    .to_utc(spec.tz)?
                    .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            )
        }),
        Transform::Number { locale, cjk_units } => map_str(items, |s| {
            values::parse_number(s, locale, *cjk_units).map(|d| d.to_string())
        }),
        Transform::Url(spec) => map_str(items, |s| values::normalize_url(s, ctx.base, spec)),
        Transform::Map { entries, drop_unmapped } => map_str(items, |s| {
            let key = s.trim().to_lowercase();
            match entries.iter().find(|(from, _)| *from == key) {
                Some((_, to)) => Some(to.clone()),
                None if *drop_unmapped => None,
                None => Some(s.to_string()),
            }
        }),
        Transform::Duration => map_str(items, |s| values::parse_duration(s).map(|n| n.to_string())),
        Transform::Decode(kind) => map_str(items, |s| values::decode(s, *kind)),
    })
}

fn slice(items: Vec<Item>, start: i32, end: i32) -> Vec<Item> {
    let len = items.len() as i64;
    let norm = |i: i32| -> i64 {
        let i = i as i64;
        if i < 0 {
            (len + i).max(0)
        } else {
            i.min(len)
        }
    };
    let s = norm(start);
    let e = if end == 0 { len } else { norm(end) };
    if s >= e {
        return Vec::new();
    }
    items.into_iter().skip(s as usize).take((e - s) as usize).collect()
}

/// TEXT normalization for items that are not already block text.
pub fn text_value(it: &Item) -> Option<String> {
    match it {
        Item::Str { s, text_ready: true } => Some(s.clone()),
        Item::Str { s, text_ready: false } => Some(normalize_text(s)),
        Item::Json(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_unique_stay_linear_and_bounded_on_a_hostile_string() {
        // Arrange: one meta value with 200k distinct comma-separated tokens
        let base = url::Url::parse("https://e.com/").unwrap();
        let budget = Budget::new(50_000_000);
        let hostile: String = (0..200_000).map(|i| format!("t{i},")).collect();
        let pipe = vec![
            Transform::Split(compile_regex(r"\s*,\s*", 1 << 20).unwrap()),
            Transform::Unique,
        ];
        let started = std::time::Instant::now();

        // Act
        let out = apply(&pipe, vec![Item::str(hostile)], &ctx(&base, &budget))
            .unwrap()
            .unwrap();

        // Assert
        assert_eq!(out.len(), MAX_PIPE_ITEMS);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
        assert!(
            budget.used() > 100_000,
            "the string's size is charged: {}",
            budget.used()
        );
    }

    fn ctx<'a>(base: &'a url::Url, budget: &'a Budget) -> PipeCtx<'a> {
        PipeCtx {
            base,
            now: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            budget,
        }
    }

    fn strs(items: &[Item]) -> Vec<&str> {
        items.iter().filter_map(Item::as_str).collect()
    }

    #[test]
    fn regex_split_unique_slice_join() {
        // Arrange
        let base = url::Url::parse("https://e.com/").unwrap();
        let budget = Budget::new(1000);
        let pipe = vec![
            Transform::Split(compile_regex(r"\s*,\s*", 1 << 20).unwrap()),
            Transform::Unique,
            Transform::Slice { start: 0, end: 2 },
            Transform::Join(" | ".into()),
        ];

        // Act
        let out = apply(&pipe, vec![Item::str("a, b, a, c")], &ctx(&base, &budget))
            .unwrap()
            .unwrap();

        // Assert
        assert_eq!(strs(&out), vec!["a | b"]);
    }

    #[test]
    fn regex_without_match_is_a_miss() {
        // Arrange
        let base = url::Url::parse("https://e.com/").unwrap();
        let budget = Budget::new(1000);
        let pipe = vec![Transform::Regex {
            re: compile_regex(r"(\d+) points", 1 << 20).unwrap(),
            group: 1,
        }];

        // Act
        let out = apply(&pipe, vec![Item::str("no score")], &ctx(&base, &budget)).unwrap();

        // Assert
        assert!(out.is_err());
    }

    #[test]
    fn lookaround_is_rejected_at_compile() {
        // Arrange / Act
        let r = compile_regex(r"^/(?!tag)", 1 << 20);

        // Assert
        assert!(r.is_err());
    }

    #[test]
    fn map_is_case_insensitive_and_can_drop() {
        // Arrange
        let base = url::Url::parse("https://e.com/").unwrap();
        let budget = Budget::new(1000);
        let pipe = vec![Transform::Map {
            entries: vec![("in stock".into(), "IN_STOCK".into())],
            drop_unmapped: true,
        }];

        // Act
        let out = apply(
            &pipe,
            vec![Item::str(" In Stock "), Item::str("Sold out")],
            &ctx(&base, &budget),
        )
        .unwrap()
        .unwrap();

        // Assert
        assert_eq!(strs(&out), vec!["IN_STOCK"]);
    }
}
