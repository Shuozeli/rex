//! Coercion of pipe output into typed schema values (engine spec §5, §8).

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use prost_reflect::{DynamicMessage, MessageDescriptor, ReflectMessage, Value};
use rex_dom::text::normalize_string;
use rex_dom::Document;
use rex_schema::{FieldSpec, RecordSpec, SchemaRegistry, StringKind, Target};
use serde_json::Value as Json;

use crate::transform::{text_value, Item};
use crate::values::{self, DatetimeSpec, UrlSpec};

/// Allow-list of the `article` HTML profile (engine spec §8.3).
const ARTICLE_TAGS: &[&str] = &[
    "p",
    "br",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "em",
    "strong",
    "b",
    "i",
    "u",
    "s",
    "sub",
    "sup",
    "a",
    "img",
    "figure",
    "figcaption",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "hr",
];

pub struct Coercer<'a> {
    pub registry: &'a SchemaRegistry,
    pub doc: &'a Document,
    pub now: DateTime<Utc>,
    /// The run's fuel: expanding a JSON-LD `@id` reference copies the referenced node, which is paid for.
    budget: std::rc::Rc<rex_dom::Budget>,
    sanitizer: ammonia::Builder<'static>,
}

impl<'a> Coercer<'a> {
    pub fn new(
        registry: &'a SchemaRegistry,
        doc: &'a Document,
        now: DateTime<Utc>,
        budget: std::rc::Rc<rex_dom::Budget>,
    ) -> Self {
        let mut b = ammonia::Builder::default();
        b.tags(ARTICLE_TAGS.iter().copied().collect::<HashSet<_>>())
            .clean_content_tags(
                ["script", "style", "noscript", "template"]
                    .into_iter()
                    .collect::<HashSet<_>>(),
            )
            .generic_attributes(["title"].into_iter().collect::<HashSet<_>>())
            .tag_attributes(
                [
                    ("a", ["href"].into_iter().collect::<HashSet<_>>()),
                    ("img", ["src", "alt"].into_iter().collect()),
                    ("td", ["colspan", "rowspan"].into_iter().collect()),
                    ("th", ["colspan", "rowspan"].into_iter().collect()),
                ]
                .into_iter()
                .collect(),
            )
            .url_schemes(["http", "https", "mailto"].into_iter().collect::<HashSet<_>>())
            .url_relative(ammonia::UrlRelative::RewriteWithBase(doc.base().clone()))
            .link_rel(None)
            .strip_comments(true);
        Self {
            registry,
            doc,
            now,
            budget,
            sanitizer: b,
        }
    }

    pub fn sanitize_html(&self, html: &str) -> String {
        self.sanitizer.clean(html).to_string().trim().to_string()
    }

    /// Coerce one item into the field's type. `Err` carries the miss reason.
    pub fn coerce(&self, item: &Item, field: &FieldSpec) -> Result<Value, String> {
        match &field.target {
            Target::Message(m) => self.coerce_message(item, m, 0).map(Value::Message),
            Target::Str(kind) => self.coerce_string(item, *kind).map(Value::String),
            Target::Timestamp => {
                let s = scalar_text(item, &["@value", "datePublished"]).ok_or("not a date value")?;
                let parsed = values::parse_datetime(&s, &DatetimeSpec::default(), self.now)
                    .ok_or_else(|| format!("unparseable date `{}`", clip(&s)))?;
                let utc = parsed.to_utc(None).ok_or("date outside range")?;
                Ok(Value::Message(timestamp(&field_message(field), utc)))
            }
            Target::Enum(e) => {
                let s = scalar_text(item, &["@id", "@value", "name"]).ok_or("not an enum value")?;
                self.registry
                    .enum_value_by_label(e, &s)
                    .map(Value::EnumNumber)
                    .ok_or_else(|| format!("`{}` is not a value of {}", clip(&s), e.full_name()))
            }
            Target::Bool => {
                let s = scalar_text(item, &["@value"]).ok_or("not a boolean")?;
                match s.trim().to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" => Ok(Value::Bool(true)),
                    "false" | "0" | "no" => Ok(Value::Bool(false)),
                    other => Err(format!("`{}` is not a boolean", clip(other))),
                }
            }
            Target::I32 | Target::I64 | Target::U32 | Target::U64 => {
                let s = scalar_text(item, &["@value"]).ok_or("not a number")?;
                let d = values::parse_number(&s, "en", false).ok_or_else(|| format!("no number in `{}`", clip(&s)))?;
                let n = d.as_i128().ok_or_else(|| format!("`{d}` is not an integer"))?;
                let out_of_range = || format!("{n} is out of range");
                Ok(match field.target {
                    Target::I32 => Value::I32(i32::try_from(n).map_err(|_| out_of_range())?),
                    Target::I64 => Value::I64(i64::try_from(n).map_err(|_| out_of_range())?),
                    Target::U32 => Value::U32(u32::try_from(n).map_err(|_| out_of_range())?),
                    _ => Value::U64(u64::try_from(n).map_err(|_| out_of_range())?),
                })
            }
            Target::F32 | Target::F64 => {
                let s = scalar_text(item, &["@value"]).ok_or("not a number")?;
                let d = values::parse_number(&s, "en", false).ok_or_else(|| format!("no number in `{}`", clip(&s)))?;
                let f = d.as_f64().ok_or("number does not fit a float")?;
                Ok(if matches!(field.target, Target::F32) {
                    Value::F32(f as f32)
                } else {
                    Value::F64(f)
                })
            }
        }
    }

    fn coerce_string(&self, item: &Item, kind: StringKind) -> Result<String, String> {
        let out = match kind {
            StringKind::String => normalize_string(
                &scalar_text(item, &["@value", "name", "text", "headline"]).ok_or("object has no text value")?,
            ),
            StringKind::Text => match item {
                Item::Json(_) => {
                    let s = scalar_text(item, &["@value", "text"]).ok_or("object has no text value")?;
                    text_value(&Item::str(s)).unwrap_or_default()
                }
                other => text_value(other).unwrap_or_default(),
            },
            StringKind::Html => self.sanitize_html(&scalar_text(item, &["@value"]).ok_or("object has no HTML value")?),
            StringKind::Url => {
                let raw = scalar_text(item, &["url", "contentUrl", "@id", "@value"]).ok_or("object has no URL")?;
                values::normalize_url(&raw, self.doc.base(), &UrlSpec::default())
                    .ok_or_else(|| format!("`{}` is not an http(s) URL", clip(&raw)))?
            }
            StringKind::Decimal => {
                let s = scalar_text(item, &["@value", "price"]).ok_or("not a number")?;
                values::parse_number(&s, "en", false)
                    .ok_or_else(|| format!("no number in `{}`", clip(&s)))?
                    .to_string()
            }
            StringKind::Date => {
                let s = scalar_text(item, &["@value"]).ok_or("not a date")?;
                values::parse_datetime(&s, &DatetimeSpec::default(), self.now)
                    .ok_or_else(|| format!("unparseable date `{}`", clip(&s)))?
                    .local_date()
                    .format("%Y-%m-%d")
                    .to_string()
            }
        };
        if out.is_empty() {
            return Err("empty after normalization".into());
        }
        Ok(out)
    }

    fn coerce_message(&self, item: &Item, desc: &MessageDescriptor, depth: u32) -> Result<DynamicMessage, String> {
        if depth > 8 {
            return Err("object nesting too deep".into());
        }
        let record = self.registry.record_of(desc).map_err(|e| e.to_string())?;
        let mut msg = DynamicMessage::new(desc.clone());
        match item {
            Item::Str { s, .. } => {
                let slot = record
                    .scalar_slot()
                    .ok_or_else(|| format!("a plain value cannot fill {}", desc.full_name()))?;
                let v = self.coerce(&Item::str(s.clone()), slot)?;
                msg.set_field(&slot.desc, v);
            }
            Item::Json(obj) => self.fill_from_object(&mut msg, &record, obj, depth)?,
        }
        for f in record.fields.iter().filter(|f| f.required) {
            if !msg.has_field(&f.desc) {
                return Err(format!("{} is missing required `{}`", desc.full_name(), f.name));
            }
        }
        if msg.fields().next().is_none() {
            return Err(format!("no field of {} could be filled", desc.full_name()));
        }
        Ok(msg)
    }

    fn fill_from_object(
        &self,
        msg: &mut DynamicMessage,
        record: &RecordSpec,
        obj: &Json,
        depth: u32,
    ) -> Result<(), String> {
        let index = self.doc.jsonld();
        for f in &record.fields {
            let key = if f.schema_org.is_empty() {
                f.name.as_str()
            } else {
                f.schema_org.as_str()
            };
            let Some(raw) = obj.get(key) else { continue };
            let values: Vec<&Json> = match raw {
                Json::Array(a) => a.iter().collect(),
                other => vec![other],
            };
            let cap = if f.max_items > 0 { f.max_items as usize } else { 1000 };
            let mut typed = Vec::new();
            for v in values {
                if typed.len() >= cap {
                    break;
                }
                let resolved = index.resolve(v);
                self.budget
                    .spend(1 + rex_dom::jsonld::value_bytes(resolved) / 16)
                    .map_err(|_| "budget exceeded expanding JSON-LD".to_string())?;
                let resolved = resolved.clone();
                let Some(it) = Item::from_json(resolved) else { continue };
                let result = match &f.target {
                    Target::Message(m) => self.coerce_message(&it, m, depth + 1).map(Value::Message),
                    _ => self.coerce(&it, f),
                };
                if let Ok(v) = result {
                    typed.push(v);
                    if !f.repeated {
                        break;
                    }
                }
            }
            if typed.is_empty() {
                continue;
            }
            if f.repeated {
                msg.set_field(&f.desc, Value::List(typed));
            } else {
                msg.set_field(&f.desc, typed.remove(0));
            }
        }
        Ok(())
    }
}

/// A string view of an item: the string itself, or the first present key of an object.
fn scalar_text(item: &Item, keys: &[&str]) -> Option<String> {
    match item {
        Item::Str { s, .. } => Some(s.clone()),
        Item::Json(v) => keys.iter().find_map(|k| match v.get(*k) {
            Some(Json::String(s)) => Some(s.clone()),
            Some(Json::Number(n)) => Some(match n.as_f64() {
                Some(f) if n.is_f64() => format!("{f}"),
                _ => n.to_string(),
            }),
            _ => None,
        }),
    }
}

fn field_message(field: &FieldSpec) -> MessageDescriptor {
    match field.desc.kind() {
        prost_reflect::Kind::Message(m) => m,
        _ => unreachable!("timestamp targets are message fields"),
    }
}

pub fn timestamp(desc: &MessageDescriptor, t: DateTime<Utc>) -> DynamicMessage {
    let mut m = DynamicMessage::new(desc.clone());
    m.set_field_by_name("seconds", Value::I64(t.timestamp()));
    let nanos = t.timestamp_subsec_nanos() as i32;
    if nanos != 0 {
        m.set_field_by_name("nanos", Value::I32(nanos));
    }
    m
}

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(60).collect();
    if s.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// String form of a typed value, for `field` references and traces.
pub fn value_text(v: &Value) -> Option<String> {
    Some(match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::I32(n) => n.to_string(),
        Value::I64(n) => n.to_string(),
        Value::U32(n) => n.to_string(),
        Value::U64(n) => n.to_string(),
        Value::F32(n) => n.to_string(),
        Value::F64(n) => n.to_string(),
        Value::EnumNumber(n) => n.to_string(),
        Value::Message(m) if m.descriptor().full_name() == "google.protobuf.Timestamp" => {
            let secs = m.get_field_by_name("seconds").and_then(|v| v.as_i64()).unwrap_or(0);
            let nanos = m.get_field_by_name("nanos").and_then(|v| v.as_i32()).unwrap_or(0);
            DateTime::from_timestamp(secs, nanos as u32)?.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
        }
        Value::Message(m) => {
            let name = m.get_field_by_name("name").and_then(|v| v.as_str().map(str::to_string));
            name.unwrap_or_else(|| format!("<{}>", m.descriptor().name()))
        }
        _ => return None,
    })
}
