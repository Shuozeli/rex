//! Evaluation (engine spec §7 "extract").

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use prost::Message as _;
use prost_reflect::{DynamicMessage, Value};
use rex_dom::text::{block_text_excluding, own_text, string_text_excluding};
use rex_dom::{Budget, BudgetExceeded, Document, Element};
use rex_proto::extract as pb;
use rex_readability::Readable;
use rex_schema::{RecordSpec, SchemaRegistry, StringKind, Target};

use crate::coerce::{timestamp, value_text, Coercer};
use crate::recipe::{natural_take, Candidate, CompiledRecipe, Rule, Source};
use crate::transform::{apply, Item, PipeCtx};

/// Host-supplied inputs. The engine has no clock: `now` drives relative dates and `extract_time`.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub now: DateTime<Utc>,
    pub fetch_time: Option<DateTime<Utc>>,
    pub hints: BTreeMap<String, String>,
    pub source_id: String,
    pub source_ref: String,
    /// Host label appended to `extractor_version`, e.g. "cli" or "ext/0.1.0".
    pub host: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub level: Level,
    pub code: &'static str,
    pub path: String,
    pub recipe: String,
    pub candidate: Option<u32>,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct Attempt {
    pub candidate: u32,
    pub recipe: String,
    pub source: String,
    pub raw: Vec<String>,
    pub outcome: String,
}

#[derive(Clone, Debug, Default)]
pub struct FieldTrace {
    pub path: String,
    pub attempts: Vec<Attempt>,
    pub winner: Option<u32>,
    pub value: Vec<String>,
}

pub struct ExtractResult {
    pub extraction: pb::Extraction,
    pub record: DynamicMessage,
    pub diagnostics: Vec<Diagnostic>,
    pub trace: Vec<FieldTrace>,
    pub fuel_used: u64,
}

/// Winning value of a rule, kept for `field` references and provenance.
#[derive(Clone, Debug)]
struct Won {
    values: Vec<Value>,
    texts: Vec<String>,
    origin: pb::FieldOrigin,
    candidate: u32,
    owner: usize,
}

struct LevelOut {
    msg: DynamicMessage,
    missing_required: Vec<String>,
    expectations_failed: Vec<String>,
    primary_missing: bool,
    origins: HashMap<String, pb::FieldOrigin>,
}

struct Run<'a> {
    recipe: &'a CompiledRecipe,
    doc: &'a Document,
    ctx: &'a Ctx,
    budget: std::rc::Rc<Budget>,
    coercer: Coercer<'a>,
    readable: Option<Option<Readable>>,
    diags: Vec<Diagnostic>,
    trace: Vec<FieldTrace>,
    sources: Vec<pb::FieldSource>,
    /// Bytes of values in the record so far, held to `Limits::max_output_bytes`.
    output_bytes: usize,
}

pub fn extract(recipe: &CompiledRecipe, registry: &SchemaRegistry, doc: &Document, ctx: &Ctx) -> ExtractResult {
    // Shared with the coercer, which pays for the JSON-LD objects it expands.
    let budget = std::rc::Rc::new(Budget::new(recipe.limits.fuel));
    let mut run = Run {
        recipe,
        doc,
        ctx,
        budget: budget.clone(),
        coercer: Coercer::new(registry, doc, ctx.now, budget),
        readable: None,
        diags: Vec::new(),
        trace: Vec::new(),
        sources: Vec::new(),
        output_bytes: 0,
    };
    let doc_root = doc.root();
    let mut root_missing = false;
    let root = match &recipe.root {
        None => doc_root,
        Some(sel) => match doc
            .select(doc_root, sel, &run.budget)
            .ok()
            .and_then(|v| v.into_iter().next())
        {
            Some(el) => el,
            None => {
                root_missing = true;
                run.diag(
                    Level::Warn,
                    "root_missing",
                    "",
                    0,
                    None,
                    "the recipe root selector matched nothing",
                );
                doc_root
            }
        },
    };

    let record_spec = recipe.record.clone();
    let outcome = run.eval_level(&recipe.rules, &record_spec, root, root, "", true);
    let (out, budget_exceeded) = match outcome {
        Ok(o) => (o, false),
        Err(BudgetExceeded) => {
            run.diag(
                Level::Warn,
                "budget_exceeded",
                "",
                0,
                None,
                "fuel budget exhausted; extraction aborted",
            );
            (
                LevelOut {
                    msg: DynamicMessage::new(record_spec.desc.clone()),
                    missing_required: Vec::new(),
                    expectations_failed: Vec::new(),
                    primary_missing: true,
                    origins: HashMap::new(),
                },
                true,
            )
        }
    };

    let status = if root_missing || out.primary_missing {
        pb::ExtractionStatus::Empty
    } else if budget_exceeded || !out.missing_required.is_empty() {
        pb::ExtractionStatus::Invalid
    } else if !out.expectations_failed.is_empty() {
        pb::ExtractionStatus::FailedExpectation
    } else {
        pb::ExtractionStatus::Ok
    };

    let agreement = if budget_exceeded {
        None
    } else {
        run.agreement(&record_spec, &out)
    };
    let ts_desc = registry
        .pool()
        .get_message_by_name("google.protobuf.Timestamp")
        .expect("well-known Timestamp");
    let to_ts = |t: DateTime<Utc>| {
        let m = timestamp(&ts_desc, t);
        prost_types::Timestamp {
            seconds: m.get_field_by_name("seconds").and_then(|v| v.as_i64()).unwrap_or(0),
            nanos: m.get_field_by_name("nanos").and_then(|v| v.as_i32()).unwrap_or(0),
        }
    };
    let rref = |r: &crate::recipe::RecipeId| pb::RecipeRef {
        id: r.id.clone(),
        version: r.version,
        sha256: r.sha256.clone(),
    };
    let extraction = pb::Extraction {
        recipe: Some(rref(&recipe.reference)),
        schema: record_spec.full_name().to_string(),
        url: doc.url().to_string(),
        extractor_version: format!("rex/{} {}", env!("CARGO_PKG_VERSION"), ctx.host)
            .trim()
            .to_string(),
        fetch_time: ctx.fetch_time.map(to_ts),
        extract_time: Some(to_ts(ctx.now)),
        source_id: ctx.source_id.clone(),
        source_ref: ctx.source_ref.clone(),
        status: status as i32,
        missing_required: out.missing_required.clone(),
        expectations_failed: out.expectations_failed.clone(),
        sources: run.sources.clone(),
        body_readability_agreement_bps: agreement,
        record: Some(prost_types::Any {
            type_url: format!("type.googleapis.com/{}", record_spec.full_name()),
            value: out.msg.encode_to_vec(),
        }),
        chain: recipe.chain.iter().map(rref).collect(),
    };
    ExtractResult {
        extraction,
        record: out.msg,
        diagnostics: run.diags,
        trace: run.trace,
        fuel_used: run.budget.used(),
    }
}

impl<'a> Run<'a> {
    fn diag(
        &mut self,
        level: Level,
        code: &'static str,
        path: &str,
        owner: usize,
        candidate: Option<u32>,
        detail: impl Into<String>,
    ) {
        let recipe = self.recipe.chain.get(owner).map(|r| r.label()).unwrap_or_default();
        self.diags.push(Diagnostic {
            level,
            code,
            path: path.to_string(),
            recipe,
            candidate,
            detail: detail.into(),
        });
    }

    fn eval_level(
        &mut self,
        rules: &[Rule],
        record: &RecordSpec,
        scope: Element<'a>,
        root: Element<'a>,
        prefix: &str,
        record_provenance: bool,
    ) -> Result<LevelOut, BudgetExceeded> {
        let mut msg = DynamicMessage::new(record.desc.clone());
        let mut won: HashMap<String, Won> = HashMap::new();
        let mut expectations_failed = Vec::new();
        let mut origins = HashMap::new();

        for rule in rules {
            let path = format!("{prefix}{}", rule.path);
            let mut trace = FieldTrace {
                path: path.clone(),
                ..Default::default()
            };
            let mut winner: Option<Won> = None;
            // Parent provenance goes before the provenance of nested `each` items.
            let source_slot = self.sources.len();

            if let Some(each) = &rule.each {
                let elements = self.doc.select(scope, &each.sel, &self.budget)?;
                let cap = cap_for(rule.field.max_items, self.recipe.limits.max_items);
                let mut items = Vec::new();
                for (i, el) in elements.into_iter().enumerate() {
                    if items.len() >= cap {
                        self.diag(
                            Level::Info,
                            "list_truncated",
                            &path,
                            each.owner,
                            None,
                            format!("kept the first {cap} items"),
                        );
                        break;
                    }
                    // Paths number the kept items (output order); a dropped item's provenance and trace go.
                    let sub_prefix = format!("{path}[{}].", items.len());
                    let (sources_before, trace_before) = (self.sources.len(), self.trace.len());
                    let sub = self.eval_level(
                        &each.rules,
                        &each.record,
                        el,
                        root,
                        &sub_prefix,
                        record_provenance && items.is_empty(),
                    )?;
                    if !sub.missing_required.is_empty() {
                        self.diag(
                            Level::Info,
                            "item_invalid",
                            &format!("{path}[{i}]"),
                            each.owner,
                            None,
                            format!("dropped: missing {}", sub.missing_required.join(", ")),
                        );
                        self.sources.truncate(sources_before);
                        self.trace.truncate(trace_before);
                        continue;
                    }
                    let mut msg = sub.msg;
                    if let Some(f) = &each.index_field {
                        let n = items.len() as u64 + 1;
                        let v = match f.kind() {
                            prost_reflect::Kind::Int32 => Value::I32(n as i32),
                            prost_reflect::Kind::Int64 => Value::I64(n as i64),
                            prost_reflect::Kind::Uint32 => Value::U32(n as u32),
                            _ => Value::U64(n),
                        };
                        msg.set_field(f, v);
                    }
                    items.push(Value::Message(msg));
                }
                trace.attempts.push(Attempt {
                    candidate: 0,
                    recipe: self.recipe.chain[each.owner].label(),
                    source: "each".into(),
                    raw: vec![format!("{} item(s)", items.len())],
                    outcome: if items.is_empty() {
                        "no items".into()
                    } else {
                        "won".into()
                    },
                });
                if !items.is_empty() {
                    winner = Some(Won {
                        texts: Vec::new(),
                        values: items,
                        origin: pb::FieldOrigin::SiteSelector,
                        candidate: 0,
                        owner: each.owner,
                    });
                }
            }

            if winner.is_none() {
                for cand in &rule.candidates {
                    let (items, origin) = self.source_items(cand, rule, scope, root, &won)?;
                    let mut attempt = Attempt {
                        candidate: cand.index,
                        recipe: self.recipe.chain[cand.owner].label(),
                        source: describe(cand),
                        raw: items.iter().take(5).map(Item::preview).collect(),
                        outcome: String::new(),
                    };
                    if items.is_empty() {
                        attempt.outcome = "no_match".into();
                        trace.attempts.push(attempt);
                        continue;
                    }
                    let pctx = PipeCtx {
                        base: self.doc.base(),
                        now: self.ctx.now,
                        budget: &self.budget,
                        doc: Some(self.doc),
                    };
                    let piped = match apply(&cand.pipe, items, &pctx)? {
                        Ok(v) => v,
                        Err(reason) => {
                            attempt.outcome = format!("transform_miss: {reason}");
                            trace.attempts.push(attempt);
                            continue;
                        }
                    };
                    let (values, first_err) = self.coerce_all(&piped, rule);
                    if values.is_empty() {
                        attempt.outcome = format!("coerce_failed: {}", first_err.unwrap_or_else(|| "empty".into()));
                        trace.attempts.push(attempt);
                        continue;
                    }
                    attempt.outcome = "won".into();
                    trace.attempts.push(attempt);
                    let origin = origin.unwrap_or(pb::FieldOrigin::Unspecified);
                    let texts = values.iter().filter_map(value_text).collect();
                    winner = Some(Won {
                        values,
                        texts,
                        origin,
                        candidate: cand.index,
                        owner: cand.owner,
                    });
                    break;
                }
            }

            if winner.is_none() {
                if let Some((owner, value)) = &rule.default {
                    if let Ok(v) = self.coercer.coerce(&Item::str(value.clone()), &rule.field) {
                        trace.attempts.push(Attempt {
                            candidate: u32::MAX,
                            recipe: self.recipe.chain[*owner].label(),
                            source: "default".into(),
                            raw: vec![value.clone()],
                            outcome: "won".into(),
                        });
                        winner = Some(Won {
                            texts: vec![value.clone()],
                            values: vec![v],
                            origin: pb::FieldOrigin::Const,
                            candidate: u32::MAX,
                            owner: *owner,
                        });
                    }
                }
            }

            if let Some(expect_owner) = rule.expect_owner {
                let own_won = winner
                    .as_ref()
                    .is_some_and(|w| w.owner == expect_owner && w.origin != pb::FieldOrigin::Const);
                if !own_won {
                    expectations_failed.push(path.clone());
                    self.diag(
                        Level::Warn,
                        "expectation_failed",
                        &path,
                        expect_owner,
                        None,
                        "none of this recipe's own candidates matched",
                    );
                }
            }

            match winner {
                None => {
                    trace.value = Vec::new();
                }
                Some(w) => {
                    trace.winner = Some(w.candidate);
                    trace.value = w
                        .values
                        .iter()
                        .take(5)
                        .filter_map(value_text)
                        .map(|s| s.chars().take(160).collect())
                        .collect();
                    let value = if rule.field.repeated {
                        Value::List(w.values.clone())
                    } else {
                        w.values[0].clone()
                    };
                    set_path(&mut msg, &rule.segments, value);
                    origins.insert(rule.path.clone(), w.origin);
                    if record_provenance {
                        self.sources.insert(
                            source_slot,
                            pb::FieldSource {
                                path: path.clone(),
                                origin: w.origin as i32,
                                candidate: if w.candidate == u32::MAX { 0 } else { w.candidate },
                                recipe: self.recipe.chain[w.owner].label(),
                            },
                        );
                    }
                    won.insert(rule.path.clone(), w);
                }
            }
            if record_provenance {
                self.trace.push(trace);
            }
        }

        let missing_required = record
            .fields
            .iter()
            .filter(|f| f.required && !msg.has_field(&f.desc))
            .map(|f| format!("{prefix}{}", f.name))
            .collect();
        let primary_missing = !record.primary.is_empty() && record.primary.iter().all(|p| !msg.has_field_by_name(p));
        Ok(LevelOut {
            msg,
            missing_required,
            expectations_failed,
            primary_missing,
            origins,
        })
    }

    fn coerce_all(&mut self, items: &[Item], rule: &Rule) -> (Vec<Value>, Option<String>) {
        let cap = cap_for(rule.field.max_items, self.recipe.limits.max_items);
        let mut out = Vec::new();
        let mut first_err = None;
        for it in items {
            if it.is_empty() {
                continue;
            }
            match self.coercer.coerce(it, &rule.field) {
                Ok(v) => {
                    let n = value_bytes(&v);
                    if self.output_bytes + n > self.recipe.limits.max_output_bytes {
                        first_err.get_or_insert_with(|| "the record reached the output size limit".to_string());
                        break;
                    }
                    self.output_bytes += n;
                    out.push(v);
                    if !rule.field.repeated || out.len() >= cap {
                        break;
                    }
                }
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        (out, first_err)
    }

    /// Raw items for a candidate, plus its origin (None when it inherits a referenced field's origin
    /// that does not exist).
    fn source_items(
        &mut self,
        cand: &Candidate,
        rule: &Rule,
        scope: Element<'a>,
        root: Element<'a>,
        won: &HashMap<String, Won>,
    ) -> Result<(Vec<Item>, Option<pb::FieldOrigin>), BudgetExceeded> {
        let doc = self.doc;
        Ok(match &cand.source {
            Source::Css {
                sel,
                take,
                attribute,
                scope: css_scope,
                exclude,
            } => {
                let base = match css_scope {
                    pb::CssScope::Root => root,
                    pb::CssScope::Document => doc.root(),
                    pb::CssScope::Unspecified | pb::CssScope::Current => scope,
                };
                let elements = doc.select_limited(base, sel, &self.budget, crate::transform::MAX_PIPE_ITEMS)?;
                let take = if *take == pb::Take::Unspecified {
                    natural_take(&rule.field)
                } else {
                    *take
                };
                let mut items = Vec::new();
                if take == pb::Take::Exists {
                    if !elements.is_empty() {
                        items.push(Item::str("true"));
                    }
                } else {
                    let mut collected = 0usize;
                    for el in elements {
                        self.budget.spend(1)?;
                        let mut skip = HashSet::new();
                        for x in exclude {
                            for node in el.descendants() {
                                self.budget.spend(1)?;
                                if let Some(d) = Element::wrap(node) {
                                    if x.matches(&d) {
                                        skip.insert(d.id());
                                    }
                                }
                            }
                        }
                        let item = match take {
                            pb::Take::BlockText => Item::text(block_text_excluding(el, &self.budget, &skip)?),
                            pb::Take::OwnText => Item::str(own_text(el)),
                            pb::Take::Html => Item::str(doc.inner_html_excluding(el, &skip)),
                            pb::Take::OuterHtml => Item::str(doc.outer_html_excluding(el, &skip)),
                            pb::Take::Attribute => match el.value().attr(attribute) {
                                Some(v) if attribute.eq_ignore_ascii_case("srcset") => Item::str(largest_srcset(v)),
                                Some(v) => Item::str(v),
                                None => continue,
                            },
                            pb::Take::Text | pb::Take::Unspecified | pb::Take::Exists => {
                                Item::str(string_text_excluding(el, &self.budget, &skip)?)
                            }
                        };
                        if let Item::Str { s, .. } = &item {
                            self.budget.spend((s.len() / 16) as u64)?; // serializing is work too
                            collected += s.len();
                        }
                        if !item.is_empty() {
                            items.push(item);
                        }
                        // Nested matches (a selector hitting 30k nested divs) would otherwise build O(n^2)
                        // bytes of HTML; more than a few times the output limit can never be used.
                        if collected > 4 * self.recipe.limits.max_output_bytes {
                            break;
                        }
                    }
                }
                (items, Some(pb::FieldOrigin::SiteSelector))
            }
            Source::JsonLd { types, path } => {
                let index = doc.jsonld();
                let mut items = Vec::new();
                let cap = cap_for(rule.field.max_items, self.recipe.limits.max_items);
                for node in index.nodes_of_types(types) {
                    self.budget.spend(1)?;
                    let values = index.eval_path_bounded(node, path, &self.budget, cap)?;
                    items = values.into_iter().filter_map(Item::from_json).collect();
                    if !items.is_empty() {
                        break;
                    }
                }
                (items, Some(pb::FieldOrigin::JsonLd))
            }
            Source::Microdata { types, path } => {
                let plain = rex_dom::jsonld::JsonLdIndex::default();
                let mut items = Vec::new();
                for item in doc.microdata() {
                    let t = item.get("@type").and_then(|v| v.as_str()).unwrap_or_default();
                    if !types.iter().any(|w| w == t) {
                        continue;
                    }
                    let cap = cap_for(rule.field.max_items, self.recipe.limits.max_items);
                    items = plain
                        .eval_path_bounded(item, path, &self.budget, cap)?
                        .into_iter()
                        .filter_map(Item::from_json)
                        .collect();
                    if !items.is_empty() {
                        break;
                    }
                }
                (items, Some(pb::FieldOrigin::Microdata))
            }
            Source::Meta(key) => {
                let k = key.to_ascii_lowercase();
                let origin = if k.starts_with("og:") || k.starts_with("article:") {
                    pb::FieldOrigin::OpenGraph
                } else {
                    pb::FieldOrigin::Meta
                };
                (
                    doc.meta(key)
                        .into_iter()
                        .map(Item::str)
                        .filter(|i| !i.is_empty())
                        .collect(),
                    Some(origin),
                )
            }
            Source::Link(rel) => (
                doc.link(rel).into_iter().map(Item::str).collect(),
                Some(pb::FieldOrigin::Meta),
            ),
            Source::Readability(part) => {
                let r = self.readable()?;
                let item = match (part, r) {
                    (_, None) => None,
                    (pb::ReadabilityPart::Title, Some(r)) => Some(Item::str(r.title)),
                    (pb::ReadabilityPart::Byline, Some(r)) => Some(Item::str(r.byline)),
                    (pb::ReadabilityPart::Excerpt, Some(r)) => Some(Item::str(r.excerpt)),
                    (pb::ReadabilityPart::ContentHtml, Some(r)) => Some(Item::str(r.content_html)),
                    (pb::ReadabilityPart::ContentText, Some(r)) => Some(Item::text(r.content_text)),
                    (pb::ReadabilityPart::Unspecified, Some(_)) => None,
                };
                (
                    item.into_iter().filter(|i| !i.is_empty()).collect(),
                    Some(pb::FieldOrigin::Readability),
                )
            }
            Source::Url {
                part,
                segment,
                query_param,
            } => {
                let u = doc.url();
                let v = match part {
                    pb::UrlPart::Href => Some(u.to_string()),
                    pb::UrlPart::Host => u.host_str().map(str::to_string),
                    pb::UrlPart::PathSegment => {
                        let segs: Vec<&str> = u
                            .path_segments()
                            .map(|s| s.filter(|x| !x.is_empty()).collect())
                            .unwrap_or_default();
                        let idx = if *segment < 0 {
                            segs.len() as i64 + *segment as i64
                        } else {
                            *segment as i64
                        };
                        usize::try_from(idx)
                            .ok()
                            .and_then(|i| segs.get(i))
                            .map(|s| s.to_string())
                    }
                    pb::UrlPart::QueryParam => u
                        .query_pairs()
                        .find(|(k, _)| k == query_param.as_str())
                        .map(|(_, v)| v.into_owned()),
                    pb::UrlPart::Unspecified => None,
                };
                (v.into_iter().map(Item::str).collect(), Some(pb::FieldOrigin::Url))
            }
            Source::Const(v) => (vec![Item::str(v.clone())], Some(pb::FieldOrigin::Const)),
            Source::Field(p) => match won.get(p) {
                Some(w) => (w.texts.iter().map(|t| Item::str(t.clone())).collect(), Some(w.origin)),
                None => (Vec::new(), None),
            },
            Source::Hint(k) => (
                self.ctx.hints.get(k).cloned().into_iter().map(Item::str).collect(),
                Some(pb::FieldOrigin::Discovery),
            ),
        })
    }

    fn readable(&mut self) -> Result<Option<Readable>, BudgetExceeded> {
        if self.readable.is_none() {
            let r = rex_readability::extract(self.doc, &self.budget)?;
            let empty = r.content_text.is_empty() && r.title.is_empty();
            self.readable = Some(if empty { None } else { Some(r) });
        }
        Ok(self.readable.clone().flatten())
    }

    /// Token-set Jaccard x 10000 between a site-selected primary long text and readability.
    fn agreement(&mut self, record: &RecordSpec, out: &LevelOut) -> Option<u32> {
        for p in &record.primary {
            let f = record.field(p)?;
            if !matches!(f.target, Target::Str(StringKind::Text))
                || out.origins.get(p) != Some(&pb::FieldOrigin::SiteSelector)
            {
                continue;
            }
            let site = out.msg.get_field_by_name(p)?.as_str()?.to_string();
            let readable = self.readable().ok()??;
            return jaccard_bps(&site, &readable.content_text);
        }
        None
    }
}

fn describe(c: &Candidate) -> String {
    if c.selector_text.is_empty() {
        c.source.describe()
    } else {
        format!("{} `{}`", c.source.describe(), c.selector_text)
    }
}

/// Approximate encoded size of a typed value, for `Limits::max_output_bytes`.
fn value_bytes(v: &Value) -> usize {
    match v {
        Value::String(s) => s.len(),
        Value::Bytes(b) => b.len(),
        Value::Message(m) => prost::Message::encoded_len(m),
        Value::List(l) => l.iter().map(value_bytes).sum(),
        _ => 8,
    }
}

fn cap_for(field_max: u32, default: u32) -> usize {
    (if field_max > 0 { field_max } else { default }) as usize
}

fn set_path(msg: &mut DynamicMessage, segments: &[String], value: Value) {
    if segments.len() == 1 {
        msg.set_field_by_name(&segments[0], value);
        return;
    }
    if let Some(Value::Message(sub)) = msg.get_field_by_name_mut(&segments[0]) {
        set_path(sub, &segments[1..], value);
    }
}

fn largest_srcset(srcset: &str) -> String {
    // The HTML srcset grammar: a URL runs to whitespace (it may contain commas, as Cloudinary's
    // "w_320,c_fill" does); a trailing comma ends it; otherwise descriptors run to the next comma.
    let mut best: Option<(f64, String)> = None;
    let mut rest = srcset;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let url_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (raw_url, after) = rest.split_at(url_end);
        let (url, descriptor, next) = if let Some(u) = raw_url.strip_suffix(',') {
            (u.trim_end_matches(','), "", after)
        } else {
            let d_end = after.find(',').unwrap_or(after.len());
            (raw_url, after[..d_end].trim(), &after[d_end..])
        };
        let w = descriptor
            .split_whitespace()
            .next()
            .and_then(|d| d.trim_end_matches(['w', 'x']).parse::<f64>().ok())
            .unwrap_or(1.0);
        if !url.is_empty() && best.as_ref().is_none_or(|(bw, _)| w > *bw) {
            best = Some((w, url.to_string()));
        }
        rest = next;
    }
    best.map(|(_, u)| u).unwrap_or_default()
}

fn tokens(s: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut word = String::new();
    for c in s.chars() {
        if rex_dom::text::is_cjk(c) {
            if !word.is_empty() {
                out.insert(std::mem::take(&mut word));
            }
            out.insert(c.to_string());
        } else if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if !word.is_empty() {
            out.insert(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.insert(word);
    }
    out
}

pub fn jaccard_bps(a: &str, b: &str) -> Option<u32> {
    let (ta, tb) = (tokens(a), tokens(b));
    let union = ta.union(&tb).count();
    if union == 0 {
        return None;
    }
    let inter = ta.intersection(&tb).count();
    Some(((inter as u64 * 10_000) / union as u64) as u32)
}

/// Value of a typed record field as protojson-ish text (used by tests and `explain`).
pub fn record_field_text(record: &DynamicMessage, name: &str) -> Option<String> {
    record.get_field_by_name(name).and_then(|v| value_text(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srcset_keeps_commas_inside_urls() {
        // Arrange
        let cases = [
            "https://c.io/w_320,c_fill/a.jpg 320w, https://c.io/w_640,c_fill/a.jpg 640w",
            "a.jpg 1x,b.jpg 2x",
            "solo.jpg",
        ];

        // Act
        let got = cases.map(largest_srcset);

        // Assert
        assert_eq!(got, ["https://c.io/w_640,c_fill/a.jpg", "b.jpg", "solo.jpg"]);
    }

    #[test]
    fn jaccard_counts_cjk_characters_as_tokens() {
        // Arrange / Act
        let same = jaccard_bps("中文 text", "text 中文");
        let half = jaccard_bps("a b", "a c");

        // Assert
        assert_eq!(same, Some(10_000));
        assert_eq!(half, Some(3_333));
    }

    #[test]
    fn srcset_picks_largest_candidate() {
        // Arrange / Act
        let u = largest_srcset("a.jpg 320w, b.jpg 1024w, c.jpg 640w");

        // Assert
        assert_eq!(u, "b.jpg");
    }
}
