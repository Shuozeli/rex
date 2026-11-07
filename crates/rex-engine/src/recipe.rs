//! Recipe parsing, the recipe set, and compilation (engine spec §7 "compile").

use std::collections::BTreeMap;
use std::sync::Arc;

use prost_reflect::{DynamicMessage, MessageDescriptor};
use rex_dom::{Limits, Sel};
use rex_proto::extract as pb;
use rex_schema::{FieldSpec, RecordSpec, SchemaRegistry, StringKind, Target};
use sha2::{Digest, Sha256};

use crate::matching::CompiledMatch;
use crate::transform::{self, Transform};

/// The recipe message type (re-exported for hosts).
pub type RecipeProto = pb::ExtractionRecipe;

/// Engine features a recipe may `require` (decisions.md U6/U8).
pub const SUPPORTED_FEATURES: &[&str] = &["relative_dates"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    pub recipe: String,
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}: {}", self.recipe, self.message)
        } else {
            write!(f, "{} [{}]: {}", self.recipe, self.path, self.message)
        }
    }
}

fn err(recipe: &str, path: impl Into<String>, message: impl Into<String>) -> CompileError {
    CompileError {
        recipe: recipe.to_string(),
        path: path.into(),
        message: message.into(),
    }
}

/// Parse a recipe from YAML (or JSON, which is YAML) strictly into the proto (unknown keys = error).
pub fn parse_recipe(text: &str, registry: &SchemaRegistry) -> Result<pb::ExtractionRecipe, String> {
    let value = parse_document(text)?;
    let desc = recipe_descriptor(registry)?;
    let dynamic = DynamicMessage::deserialize(desc, value)
        .map_err(|e| format!("does not match rex.extract.v1.ExtractionRecipe: {e}"))?;
    dynamic
        .transcode_to::<pb::ExtractionRecipe>()
        .map_err(|e| format!("transcode failed: {e}"))
}

#[cfg(feature = "yaml")]
fn parse_document(text: &str) -> Result<serde_json::Value, String> {
    serde_yaml::from_str(text).map_err(|e| format!("not valid YAML/JSON: {e}"))
}

#[cfg(not(feature = "yaml"))]
fn parse_document(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("not valid JSON (this build has no YAML support): {e}"))
}

fn recipe_descriptor(registry: &SchemaRegistry) -> Result<MessageDescriptor, String> {
    registry
        .pool()
        .get_message_by_name(rex_proto::EXTRACTION_RECIPE)
        .ok_or_else(|| "descriptor pool lacks rex.extract.v1.ExtractionRecipe".to_string())
}

/// Canonical protojson (field-number order, defaults omitted) and its sha256.
pub fn canonical_json(recipe: &pb::ExtractionRecipe, registry: &SchemaRegistry) -> Result<(String, String), String> {
    let mut dynamic = DynamicMessage::new(recipe_descriptor(registry)?);
    dynamic
        .transcode_from(recipe)
        .map_err(|e| format!("transcode failed: {e}"))?;
    let json = serde_json::to_string(&dynamic).map_err(|e| format!("serialize failed: {e}"))?;
    let digest = Sha256::digest(json.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok((json, hex))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeId {
    pub id: String,
    pub version: u32,
    pub sha256: String,
}

impl RecipeId {
    pub fn label(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

/// All recipes known to a host (base and site recipes).
#[derive(Clone)]
pub struct RecipeSet {
    registry: SchemaRegistry,
    recipes: BTreeMap<String, (Arc<pb::ExtractionRecipe>, String)>,
}

impl RecipeSet {
    pub fn new(registry: SchemaRegistry) -> Self {
        Self {
            registry,
            recipes: BTreeMap::new(),
        }
    }

    pub fn registry(&self) -> &SchemaRegistry {
        &self.registry
    }

    pub fn add(&mut self, recipe: pb::ExtractionRecipe) -> Result<(), CompileError> {
        if recipe.id.trim().is_empty() {
            return Err(err("<unnamed>", "id", "recipe id is empty"));
        }
        if self.recipes.contains_key(&recipe.id) {
            return Err(err(&recipe.id, "id", "duplicate recipe id"));
        }
        let (_, sha) = canonical_json(&recipe, &self.registry).map_err(|m| err(&recipe.id, "", m))?;
        self.recipes.insert(recipe.id.clone(), (Arc::new(recipe), sha));
        Ok(())
    }

    pub fn add_yaml(&mut self, text: &str) -> Result<String, CompileError> {
        let recipe = parse_recipe(text, &self.registry).map_err(|m| err("<parse>", "", m))?;
        let id = recipe.id.clone();
        self.add(recipe)?;
        Ok(id)
    }

    pub fn get(&self, id: &str) -> Option<&pb::ExtractionRecipe> {
        self.recipes.get(id).map(|(r, _)| r.as_ref())
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.recipes.keys().map(String::as_str)
    }

    /// A base recipe has no URL match rules (it is used through `extends` or the schema.org fallback).
    pub fn is_base(&self, id: &str) -> bool {
        self.get(id).is_some_and(is_base_recipe)
    }

    pub fn reference(&self, id: &str) -> Option<RecipeId> {
        self.recipes.get(id).map(|(r, sha)| RecipeId {
            id: r.id.clone(),
            version: r.version,
            sha256: sha.clone(),
        })
    }

    pub fn compile(&self, id: &str, limits: &Limits) -> Result<CompiledRecipe, Vec<CompileError>> {
        Compiler {
            set: self,
            limits,
            errors: Vec::new(),
        }
        .compile(id)
    }
}

pub fn is_base_recipe(r: &pb::ExtractionRecipe) -> bool {
    r.r#match
        .as_ref()
        .and_then(|m| m.urls.as_ref())
        .is_none_or(|u| u.hosts.is_empty())
}

// ---------------------------------------------------------------------------------------------
// Compiled form
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Source {
    Css {
        sel: Sel,
        take: pb::Take,
        attribute: String,
        scope: pb::CssScope,
        exclude: Vec<scraper::Selector>,
    },
    JsonLd {
        types: Vec<String>,
        path: String,
    },
    Microdata {
        types: Vec<String>,
        path: String,
    },
    Meta(String),
    Link(String),
    Readability(pb::ReadabilityPart),
    Url {
        part: pb::UrlPart,
        segment: i32,
        query_param: String,
    },
    Const(String),
    Field(String),
    Hint(String),
}

impl Source {
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Css { .. } => "css",
            Source::JsonLd { .. } => "json_ld",
            Source::Microdata { .. } => "microdata",
            Source::Meta(_) => "meta",
            Source::Link(_) => "link",
            Source::Readability(_) => "readability",
            Source::Url { .. } => "url",
            Source::Const(_) => "constant",
            Source::Field(_) => "field",
            Source::Hint(_) => "hint",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Source::Css { take, attribute, .. } => {
                let take = take.as_str_name();
                if attribute.is_empty() {
                    format!("css {take}")
                } else {
                    format!("css {take} @{attribute}")
                }
            }
            Source::JsonLd { types, path } => format!("json_ld {}:{path}", types.join("|")),
            Source::Microdata { types, path } => format!("microdata {}:{path}", types.join("|")),
            Source::Meta(k) => format!("meta {k}"),
            Source::Link(r) => format!("link {r}"),
            Source::Readability(p) => format!("readability {}", p.as_str_name()),
            Source::Url { part, .. } => format!("url {}", part.as_str_name()),
            Source::Const(v) => format!("constant {v:?}"),
            Source::Field(p) => format!("field {p}"),
            Source::Hint(k) => format!("hint {k}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    /// Index of the contributing recipe in the chain (0 = the compiled recipe itself).
    pub owner: usize,
    /// Position in the merged candidate list.
    pub index: u32,
    pub source: Source,
    /// Candidate-local transforms followed by the owning rule's transforms.
    pub pipe: Vec<Transform>,
    /// The CSS selector source text (for explain/provenance).
    pub selector_text: String,
}

#[derive(Clone, Debug)]
pub struct Rule {
    /// Dotted path relative to the record this rule belongs to.
    pub path: String,
    pub segments: Vec<String>,
    /// Spec of the leaf field.
    pub field: FieldSpec,
    pub candidates: Vec<Candidate>,
    /// Chain index whose own candidates must win, if any recipe set `expect` on this rule.
    pub expect_owner: Option<usize>,
    pub default: Option<(usize, String)>,
    pub each: Option<Each>,
}

#[derive(Clone, Debug)]
pub struct Each {
    pub owner: usize,
    pub sel: Sel,
    pub record: Arc<RecordSpec>,
    pub rules: Vec<Rule>,
    /// Integer item field that receives the 1-based position among kept items (`each_index`).
    pub index_field: Option<prost_reflect::FieldDescriptor>,
}

#[derive(Clone, Debug)]
pub struct CompiledRecipe {
    pub reference: RecipeId,
    /// The extends chain, most specific first (chain[0] == reference).
    pub chain: Vec<RecipeId>,
    pub record: Arc<RecordSpec>,
    pub root: Option<Sel>,
    pub rules: Vec<Rule>,
    pub matcher: Option<CompiledMatch>,
    pub requires: Vec<String>,
    pub limits: Limits,
}

// ---------------------------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------------------------

/// One layer of rules from one recipe in the chain.
struct Layer<'a> {
    owner: usize,
    rules: &'a [pb::FieldRule],
}

struct Compiler<'s> {
    set: &'s RecipeSet,
    limits: &'s Limits,
    errors: Vec<CompileError>,
}

impl<'s> Compiler<'s> {
    fn compile(mut self, id: &str) -> Result<CompiledRecipe, Vec<CompileError>> {
        let Some(recipe) = self.set.get(id) else {
            return Err(vec![err(id, "", "unknown recipe id")]);
        };
        // Resolve the extends chain (most specific first).
        let mut chain: Vec<&pb::ExtractionRecipe> = vec![recipe];
        let mut cur = recipe;
        while !cur.extends.is_empty() {
            if chain.len() > self.limits.max_extends_depth {
                return Err(vec![err(
                    id,
                    "extends",
                    format!("extends chain deeper than {}", self.limits.max_extends_depth),
                )]);
            }
            let Some(base) = self.set.get(&cur.extends) else {
                return Err(vec![err(
                    &cur.id,
                    "extends",
                    format!("unknown base recipe `{}`", cur.extends),
                )]);
            };
            if chain.iter().any(|r| r.id == base.id) {
                return Err(vec![err(id, "extends", "extends chain has a cycle")]);
            }
            if base.schema != recipe.schema {
                return Err(vec![err(
                    &cur.id,
                    "extends",
                    format!(
                        "base `{}` targets schema `{}`, not `{}`",
                        base.id, base.schema, recipe.schema
                    ),
                )]);
            }
            chain.push(base);
            cur = base;
        }
        let record = match self.set.registry.record(&recipe.schema) {
            Ok(r) => Arc::new(r),
            Err(e) => return Err(vec![err(id, "schema", e.to_string())]),
        };
        for r in &chain {
            for f in &r.requires {
                if !SUPPORTED_FEATURES.contains(&f.as_str()) {
                    self.errors
                        .push(err(&r.id, "requires", format!("engine does not support feature `{f}`")));
                }
            }
            // Relative dates resolve against an injected clock; hosts without it must be able to skip the recipe.
            if uses_relative_dates(&r.fields) && !r.requires.iter().any(|f| f == "relative_dates") {
                self.errors.push(err(
                    &r.id,
                    "requires",
                    "a `datetime` with `relative: true` needs `requires: [relative_dates]`",
                ));
            }
        }
        let root = if recipe.root.is_empty() {
            None
        } else {
            match Sel::parse(&recipe.root, self.limits.max_selector_len) {
                Ok(s) => Some(s),
                Err(m) => {
                    self.errors.push(err(id, "root", m));
                    None
                }
            }
        };
        let layers: Vec<Layer<'_>> = chain
            .iter()
            .enumerate()
            .rev()
            .map(|(owner, r)| Layer {
                owner,
                rules: &r.fields,
            })
            .collect();
        let rule_count: usize = chain.iter().map(|r| count_rules(&r.fields)).sum();
        if rule_count > self.limits.max_rules {
            self.errors.push(err(
                id,
                "fields",
                format!("{rule_count} rules, over the {}-rule limit", self.limits.max_rules),
            ));
        }
        let ids: Vec<String> = chain.iter().map(|r| r.id.clone()).collect();
        let rules = self.merge_level(&ids, &record, &layers, "", 0);
        let matcher = match recipe.r#match.as_ref() {
            Some(m) if !is_base_recipe(recipe) => match CompiledMatch::compile(m, self.limits) {
                Ok(c) => Some(c),
                Err(e) => {
                    self.errors.push(err(id, "match", e));
                    None
                }
            },
            _ => None,
        };
        if !self.errors.is_empty() {
            return Err(self.errors);
        }
        let chain_refs: Vec<RecipeId> = chain.iter().filter_map(|r| self.set.reference(&r.id)).collect();
        Ok(CompiledRecipe {
            reference: chain_refs[0].clone(),
            chain: chain_refs,
            record,
            root,
            rules,
            matcher,
            requires: recipe.requires.clone(),
            limits: self.limits.clone(),
        })
    }

    /// Merge one record level across the chain (layers ordered base -> most specific).
    fn merge_level(
        &mut self,
        ids: &[String],
        record: &Arc<RecordSpec>,
        layers: &[Layer<'_>],
        prefix: &str,
        depth: u32,
    ) -> Vec<Rule> {
        if depth > self.limits.max_depth {
            self.errors.push(err(&ids[0], prefix, "nesting deeper than the limit"));
            return Vec::new();
        }
        // path -> (merged candidates, expect owner, default, each layers)
        struct Acc<'a> {
            candidates: Vec<(usize, &'a pb::Candidate, &'a [pb::Transform])>,
            expect_owner: Option<usize>,
            default: Option<(usize, String)>,
            each: Option<(usize, String, String)>,
            nested: Vec<(usize, &'a [pb::FieldRule])>,
            /// `replace_inherited` with nothing of its own: the site has no good source, the field stays unset.
            omitted: bool,
        }
        let mut order: Vec<String> = Vec::new();
        let mut acc: BTreeMap<String, Acc<'_>> = BTreeMap::new();
        for layer in layers {
            let mut seen_here: Vec<&str> = Vec::new();
            for rule in layer.rules {
                let recipe_id = &ids[layer.owner];
                let full = format!("{prefix}{}", rule.path);
                if rule.path.trim().is_empty() {
                    self.errors.push(err(recipe_id, &full, "empty rule path"));
                    continue;
                }
                if seen_here.contains(&rule.path.as_str()) {
                    self.errors.push(err(recipe_id, &full, "duplicate path at this level"));
                    continue;
                }
                seen_here.push(&rule.path);
                let entry = acc.entry(rule.path.clone()).or_insert_with(|| {
                    order.push(rule.path.clone());
                    Acc {
                        candidates: Vec::new(),
                        expect_owner: None,
                        default: None,
                        each: None,
                        nested: Vec::new(),
                        omitted: false,
                    }
                });
                let own: Vec<_> = rule
                    .candidates
                    .iter()
                    .map(|c| (layer.owner, c, rule.pipe.as_slice()))
                    .collect();
                if rule.replace_inherited {
                    // Everything inherited goes: candidates, a default, and an `each` the base declared.
                    entry.omitted =
                        own.is_empty() && rule.each.is_empty() && rule.fields.is_empty() && rule.pipe.is_empty();
                    entry.candidates = own;
                    entry.default = None;
                    entry.each = None;
                    entry.nested.clear();
                } else {
                    let mut merged = own;
                    merged.append(&mut entry.candidates);
                    entry.candidates = merged;
                }
                // `expect` is not inherited: it only constrains the recipe that states it.
                entry.expect_owner = rule.expect.then_some(layer.owner);
                for t in &rule.pipe {
                    if let Some(pb::transform::Op::Default(d)) = &t.op {
                        entry.default = Some((layer.owner, d.value.clone()));
                    }
                }
                if !rule.each.is_empty() {
                    entry.each = Some((layer.owner, rule.each.clone(), rule.each_index.clone()));
                } else if !rule.each_index.is_empty() {
                    self.errors.push(err(recipe_id, &full, "`each_index` requires `each`"));
                }
                if !rule.fields.is_empty() {
                    if rule.each.is_empty() && entry.each.is_none() {
                        self.errors
                            .push(err(recipe_id, &full, "nested `fields` require `each`"));
                    }
                    entry.nested.push((layer.owner, &rule.fields));
                }
            }
        }

        let mut rules = Vec::new();
        for path in order {
            let a = acc.remove(&path).expect("accumulated path");
            let full = format!("{prefix}{path}");
            let owner_id = |o: usize| ids[o].clone();
            let segments: Vec<String> = path.split('.').map(str::to_string).collect();
            let Some(field) = self.resolve_path(&ids[0], record, &segments, &full) else {
                continue;
            };
            if a.omitted && a.candidates.is_empty() {
                continue;
            }
            let mut candidates = Vec::new();
            for (index, (owner, cand, rule_pipe)) in a.candidates.iter().enumerate() {
                let loc = format!("{full}.candidates[{index}]");
                let Some(source) = self.compile_source(&owner_id(*owner), cand, record, &segments, &field, &loc) else {
                    continue;
                };
                let mut pipe = Vec::new();
                for (ti, t) in cand.pipe.iter().enumerate() {
                    match transform::compile(t, self.limits.max_regex_bytes) {
                        Ok(transform::Compiled::Op(op)) => pipe.push(op),
                        Ok(transform::Compiled::Default(_)) => self.errors.push(err(
                            &owner_id(*owner),
                            format!("{loc}.pipe[{ti}]"),
                            "`default` is only allowed in a rule pipe",
                        )),
                        Err(m) => self.errors.push(err(&owner_id(*owner), format!("{loc}.pipe[{ti}]"), m)),
                    }
                }
                for (ti, t) in rule_pipe.iter().enumerate() {
                    match transform::compile(t, self.limits.max_regex_bytes) {
                        Ok(transform::Compiled::Op(op)) => pipe.push(op),
                        Ok(transform::Compiled::Default(_)) => {}
                        Err(m) => self
                            .errors
                            .push(err(&owner_id(*owner), format!("{full}.pipe[{ti}]"), m)),
                    }
                }
                let selector_text = match &cand.source {
                    Some(pb::candidate::Source::Css(c)) => c.selector.clone(),
                    _ => String::new(),
                };
                candidates.push(Candidate {
                    owner: *owner,
                    index: index as u32,
                    source,
                    pipe,
                    selector_text,
                });
            }
            let each = match &a.each {
                None => None,
                Some((owner, sel_text, index_name)) => {
                    let target = match &field.target {
                        Target::Message(m) if field.repeated => Some(m.clone()),
                        _ => None,
                    };
                    match target {
                        None => {
                            self.errors.push(err(
                                &owner_id(*owner),
                                &full,
                                "`each` is only valid on repeated message fields",
                            ));
                            None
                        }
                        Some(m) => match (
                            Sel::parse(sel_text, self.limits.max_selector_len),
                            self.set.registry.record_of(&m),
                        ) {
                            (Ok(sel), Ok(item_record)) => {
                                let item_record = Arc::new(item_record);
                                let nested_layers: Vec<Layer<'_>> =
                                    a.nested.iter().map(|(o, r)| Layer { owner: *o, rules: r }).collect();
                                let rules =
                                    self.merge_level(ids, &item_record, &nested_layers, &format!("{full}."), depth + 1);
                                let index_field = if index_name.is_empty() {
                                    None
                                } else {
                                    use prost_reflect::Kind;
                                    let f = item_record.desc.get_field_by_name(index_name).filter(|f| {
                                        !f.is_list()
                                            && matches!(
                                                f.kind(),
                                                Kind::Int32 | Kind::Int64 | Kind::Uint32 | Kind::Uint64
                                            )
                                    });
                                    if f.is_none() {
                                        self.errors.push(err(
                                            &owner_id(*owner),
                                            format!("{full}.each_index"),
                                            format!(
                                                "`{index_name}` is not an integer field of {}",
                                                item_record.full_name()
                                            ),
                                        ));
                                    }
                                    f
                                };
                                Some(Each {
                                    owner: *owner,
                                    sel,
                                    record: item_record,
                                    rules,
                                    index_field,
                                })
                            }
                            (Err(m), _) => {
                                self.errors.push(err(&owner_id(*owner), format!("{full}.each"), m));
                                None
                            }
                            (_, Err(e)) => {
                                self.errors.push(err(&owner_id(*owner), &full, e.to_string()));
                                None
                            }
                        },
                    }
                }
            };
            if candidates.is_empty() && each.is_none() && a.default.is_none() {
                self.errors.push(err(
                    &ids[0],
                    &full,
                    "rule has no candidates, no `each` and no `default`",
                ));
            }
            if each.is_none() && matches!(field.target, Target::Message(_)) && field.repeated {
                // Repeated message without `each`: values must be objects or scalars fitting the item record.
                if let Target::Message(m) = &field.target {
                    if let Ok(r) = self.set.registry.record_of(m) {
                        let scalar_ok = r.scalar_slot().is_some();
                        let only_scalar_sources = candidates
                            .iter()
                            .all(|c| !matches!(c.source, Source::JsonLd { .. } | Source::Microdata { .. }));
                        if !scalar_ok && only_scalar_sources && !candidates.is_empty() {
                            self.errors.push(err(&ids[0], &full, "message field needs `each`, json_ld/microdata objects, or a record with a single required string field"));
                        }
                    }
                }
            }
            if let Some((owner, value)) = &a.default {
                if let Err(m) = check_default(self.set.registry(), &field, value) {
                    self.errors.push(err(
                        &owner_id(*owner),
                        &full,
                        format!("`default` {value:?} does not fit the field: {m}"),
                    ));
                }
            }
            rules.push(Rule {
                path: path.clone(),
                segments,
                field,
                candidates,
                expect_owner: a.expect_owner,
                default: a.default,
                each,
            });
        }
        self.order_rules(&ids[0], prefix, record, rules)
    }

    fn resolve_path(
        &mut self,
        recipe: &str,
        record: &RecordSpec,
        segments: &[String],
        full: &str,
    ) -> Option<FieldSpec> {
        let mut current = record.clone();
        for (i, seg) in segments.iter().enumerate() {
            let Some(f) = current.field(seg).cloned() else {
                self.errors.push(err(
                    recipe,
                    full,
                    format!("`{seg}` is not a field of {}", current.full_name()),
                ));
                return None;
            };
            if i + 1 == segments.len() {
                return Some(f);
            }
            match &f.target {
                Target::Message(m) if !f.repeated => match self.set.registry.record_of(m) {
                    Ok(r) => current = r,
                    Err(e) => {
                        self.errors.push(err(recipe, full, e.to_string()));
                        return None;
                    }
                },
                Target::Message(_) => {
                    self.errors.push(err(
                        recipe,
                        full,
                        format!("`{seg}` is repeated: address its fields with `each` + nested rules"),
                    ));
                    return None;
                }
                _ => {
                    self.errors
                        .push(err(recipe, full, format!("`{seg}` is not a message field")));
                    return None;
                }
            }
        }
        None
    }

    /// schema.org property path for a dotted rule path (each segment must be bound).
    fn schema_org_path(&self, record: &RecordSpec, segments: &[String]) -> Option<String> {
        let mut current = record.clone();
        let mut out = Vec::new();
        for seg in segments {
            let f = current.field(seg)?.clone();
            if f.schema_org.is_empty() {
                return None;
            }
            out.push(f.schema_org.clone());
            if let Target::Message(m) = &f.target {
                current = self.set.registry.record_of(m).ok()?;
            }
        }
        Some(out.join("."))
    }

    fn compile_source(
        &mut self,
        recipe: &str,
        cand: &pb::Candidate,
        record: &RecordSpec,
        segments: &[String],
        field: &FieldSpec,
        loc: &str,
    ) -> Option<Source> {
        use pb::candidate::Source as S;
        let Some(src) = &cand.source else {
            self.errors.push(err(recipe, loc, "candidate has no source"));
            return None;
        };
        let default_types = || record.schema_org_types();
        Some(match src {
            S::Css(c) => {
                let sel = match Sel::parse(&c.selector, self.limits.max_selector_len) {
                    Ok(s) => s,
                    Err(m) => {
                        self.errors.push(err(recipe, loc, m));
                        return None;
                    }
                };
                let take = pb::Take::try_from(c.take).unwrap_or(pb::Take::Unspecified);
                if take == pb::Take::Attribute && c.attribute.trim().is_empty() {
                    self.errors.push(err(recipe, loc, "TAKE_ATTRIBUTE needs `attribute`"));
                }
                if take == pb::Take::Exists && !matches!(field.target, Target::Bool) {
                    self.errors
                        .push(err(recipe, loc, "TAKE_EXISTS is only valid for bool fields"));
                }
                let scope = pb::CssScope::try_from(c.scope).unwrap_or(pb::CssScope::Unspecified);
                let mut exclude = Vec::new();
                for (i, x) in c.exclude.iter().enumerate() {
                    if x.trim().is_empty() || x.len() > self.limits.max_selector_len {
                        self.errors.push(err(
                            recipe,
                            format!("{loc}.css.exclude[{i}]"),
                            "exclude selector is empty or too long",
                        ));
                        continue;
                    }
                    match scraper::Selector::parse(x) {
                        Ok(s) => exclude.push(s),
                        Err(e) => self.errors.push(err(
                            recipe,
                            format!("{loc}.css.exclude[{i}]"),
                            format!("invalid CSS selector `{x}`: {e:?}"),
                        )),
                    }
                }
                Source::Css {
                    sel,
                    take,
                    attribute: c.attribute.trim().to_string(),
                    scope,
                    exclude,
                }
            }
            S::JsonLd(j) => {
                let types = if j.types.is_empty() {
                    default_types()
                } else {
                    j.types.clone()
                };
                let path = if j.path.is_empty() {
                    self.schema_org_path(record, segments)
                } else {
                    Some(j.path.clone())
                };
                let Some(path) = path else {
                    self.errors.push(err(
                        recipe,
                        loc,
                        "json_ld without `path` needs a schema.org binding on every path segment",
                    ));
                    return None;
                };
                if let Err(m) = rex_dom::jsonld::validate_path(&path) {
                    self.errors.push(err(recipe, loc, m));
                }
                Source::JsonLd { types, path }
            }
            S::Microdata(m) => {
                let types = if m.types.is_empty() {
                    default_types()
                } else {
                    m.types.clone()
                };
                let path = if m.path.is_empty() {
                    self.schema_org_path(record, segments)
                } else {
                    Some(m.path.clone())
                };
                let Some(path) = path else {
                    self.errors.push(err(
                        recipe,
                        loc,
                        "microdata without `path` needs a schema.org binding on every path segment",
                    ));
                    return None;
                };
                if let Err(m) = rex_dom::jsonld::validate_path(&path) {
                    self.errors.push(err(recipe, loc, m));
                }
                Source::Microdata { types, path }
            }
            S::Meta(m) if m.key.trim().is_empty() => {
                self.errors.push(err(recipe, loc, "meta needs `key`"));
                return None;
            }
            S::Meta(m) => Source::Meta(m.key.clone()),
            S::Link(l) if l.rel.trim().is_empty() => {
                self.errors.push(err(recipe, loc, "link needs `rel`"));
                return None;
            }
            S::Link(l) => Source::Link(l.rel.clone()),
            S::Readability(r) => {
                let part = pb::ReadabilityPart::try_from(r.part).unwrap_or(pb::ReadabilityPart::Unspecified);
                if part == pb::ReadabilityPart::Unspecified {
                    self.errors.push(err(recipe, loc, "readability needs `part`"));
                    return None;
                }
                Source::Readability(part)
            }
            S::Url(u) => {
                let part = pb::UrlPart::try_from(u.part).unwrap_or(pb::UrlPart::Unspecified);
                match part {
                    pb::UrlPart::Unspecified => {
                        self.errors.push(err(recipe, loc, "url needs `part`"));
                        return None;
                    }
                    pb::UrlPart::QueryParam if u.query_param.is_empty() => {
                        self.errors
                            .push(err(recipe, loc, "URL_PART_QUERY_PARAM needs `query_param`"));
                        return None;
                    }
                    _ => {}
                }
                Source::Url {
                    part,
                    segment: u.segment,
                    query_param: u.query_param.clone(),
                }
            }
            S::Constant(c) => Source::Const(c.value.clone()),
            S::Field(f) => Source::Field(f.path.clone()),
            S::Hint(h) if h.key.trim().is_empty() => {
                self.errors.push(err(recipe, loc, "hint needs `key`"));
                return None;
            }
            S::Hint(h) => Source::Hint(h.key.clone()),
        })
    }

    /// Topological order by `field` references (references resolve among sibling rules).
    fn order_rules(&mut self, recipe: &str, prefix: &str, record: &RecordSpec, rules: Vec<Rule>) -> Vec<Rule> {
        let paths: Vec<String> = rules.iter().map(|r| r.path.clone()).collect();
        let deps: Vec<Vec<usize>> = rules
            .iter()
            .map(|r| {
                r.candidates
                    .iter()
                    .filter_map(|c| match &c.source {
                        Source::Field(p) => Some(p.clone()),
                        _ => None,
                    })
                    .filter_map(|p| {
                        let found = paths.iter().position(|x| *x == p);
                        if found.is_none() {
                            self.errors.push(err(
                                recipe,
                                format!("{prefix}{}", r.path),
                                format!("field reference `{p}` is not a sibling rule"),
                            ));
                        }
                        found
                    })
                    .collect()
            })
            .collect();
        // Kahn's algorithm, ties broken by schema field number for a stable, readable order.
        let field_no = |r: &Rule| {
            record
                .field(&r.segments[0])
                .map(|f| f.desc.number())
                .unwrap_or(u32::MAX)
        };
        let mut remaining: Vec<usize> = (0..rules.len()).collect();
        remaining.sort_by_key(|&i| (field_no(&rules[i]), i));
        let mut done: Vec<usize> = Vec::new();
        while !remaining.is_empty() {
            let pos = remaining.iter().position(|&i| deps[i].iter().all(|d| done.contains(d)));
            match pos {
                Some(p) => done.push(remaining.remove(p)),
                None => {
                    self.errors.push(err(
                        recipe,
                        prefix.trim_end_matches('.'),
                        "field references form a cycle",
                    ));
                    break;
                }
            }
        }
        let mut slots: Vec<Option<Rule>> = rules.into_iter().map(Some).collect();
        done.into_iter().filter_map(|i| slots[i].take()).collect()
    }
}

fn count_rules(rules: &[pb::FieldRule]) -> usize {
    rules.iter().map(|r| 1 + count_rules(&r.fields)).sum()
}

/// Is this string kind multi-line text (TEXT) — used for TAKE_UNSPECIFIED's natural mode.
pub fn natural_take(field: &FieldSpec) -> pb::Take {
    match field.target {
        Target::Str(StringKind::Html) => pb::Take::Html,
        Target::Str(StringKind::Text) => pb::Take::BlockText,
        Target::Bool => pb::Take::Exists,
        _ => pb::Take::Text,
    }
}

/// True if any rule (or nested rule) parses dates with `relative: true`.
fn uses_relative_dates(rules: &[pb::FieldRule]) -> bool {
    let relative = |pipe: &[pb::Transform]| {
        pipe.iter()
            .any(|t| matches!(&t.op, Some(pb::transform::Op::Datetime(d)) if d.relative))
    };
    rules
        .iter()
        .any(|r| relative(&r.pipe) || r.candidates.iter().any(|c| relative(&c.pipe)) || uses_relative_dates(&r.fields))
}

/// Coerces a `default` value into its field at compile time, so a default that can never fit is an error
/// rather than a value silently dropped on every page.
fn check_default(registry: &SchemaRegistry, field: &FieldSpec, value: &str) -> Result<(), String> {
    let doc = rex_dom::Document::parse("<html></html>", "https://default.invalid/", &Limits::default())
        .map_err(|e| e.to_string())?;
    let budget = std::rc::Rc::new(rex_dom::Budget::new(10_000));
    // No clock in wasm builds; a default is never a relative date, so any fixed instant will do.
    let epoch = chrono::DateTime::from_timestamp(0, 0).expect("epoch");
    let coercer = crate::coerce::Coercer::new(registry, &doc, epoch, budget);
    coercer.coerce(&transform::Item::str(value), field).map(|_| ())
}
