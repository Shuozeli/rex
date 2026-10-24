//! The rex engine (engine spec): compile `ExtractionRecipe`s, match pages to recipes, and turn a
//! parsed [`Document`] into a typed [`rex_proto::extract::Extraction`].

use std::collections::BTreeMap;
use std::sync::Arc;

use prost_reflect::{DynamicMessage, SerializeOptions};
use rex_dom::{Budget, Document, Limits};
use rex_proto::extract as pb;
use rex_schema::SchemaRegistry;
use url::Url;

pub mod coerce;
pub mod eval;
pub mod fixtures;
pub mod matching;
pub mod recipe;
pub mod transform;
pub mod values;

pub use eval::{Ctx, Diagnostic, ExtractResult, FieldTrace, Level};
pub use matching::UrlHit;
pub use recipe::{CompileError, CompiledRecipe, RecipeSet};
pub use rex_dom;

/// Fuel for page-stage matching (selector probes), separate from extraction fuel.
const MATCH_FUEL: u64 = 1_000_000;

/// How a page was matched to a recipe (decisions.md U2).
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// Stage 1 hits, ranked.
    pub url_hits: Vec<(String, UrlHit)>,
    /// Stage 2 confirmations, ranked.
    pub confirmed: Vec<String>,
    /// schema.org types the page declares (for the fallback).
    pub detected_types: Vec<String>,
    /// The generic recipe chosen by the schema.org fallback, if no site recipe was confirmed.
    pub fallback: Option<String>,
    /// The recipe to run.
    pub chosen: Option<String>,
}

/// A loaded, fully compiled recipe set.
pub struct Engine {
    set: RecipeSet,
    limits: Limits,
    compiled: BTreeMap<String, Arc<CompiledRecipe>>,
    /// schema.org type -> (fallback_rank, base recipe), built once (decisions.md U2, U36).
    fallback: std::collections::HashMap<String, (i32, String)>,
}

impl Engine {
    /// Compiles every recipe in the set; any compile error fails construction.
    pub fn new(set: RecipeSet, limits: Limits) -> Result<Engine, Vec<CompileError>> {
        let mut compiled = BTreeMap::new();
        let mut errors = Vec::new();
        let ids: Vec<String> = set.ids().map(str::to_string).collect();
        for id in ids {
            match set.compile(&id, &limits) {
                Ok(c) => {
                    compiled.insert(id, Arc::new(c));
                }
                Err(mut e) => errors.append(&mut e),
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        // For each type, the first schema (sorted by name) that maps it and has a base recipe.
        let mut fallback = std::collections::HashMap::new();
        let reg = set.registry();
        for schema in reg.schemas() {
            let Ok(record) = reg.record(&schema) else { continue };
            let base = compiled
                .iter()
                .find(|(id, c)| c.record.full_name() == schema && set.is_base(id))
                .map(|(id, _)| id.clone());
            let Some(base) = base else { continue };
            for t in record.schema_org_types() {
                fallback.entry(t).or_insert((record.fallback_rank, base.clone()));
            }
        }
        Ok(Engine {
            set,
            limits,
            compiled,
            fallback,
        })
    }

    pub fn recipes(&self) -> &RecipeSet {
        &self.set
    }

    pub fn registry(&self) -> &SchemaRegistry {
        self.set.registry()
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    pub fn compiled(&self, id: &str) -> Option<&Arc<CompiledRecipe>> {
        self.compiled.get(id)
    }

    /// Stage 1 over every site recipe.
    pub fn match_url(&self, url: &Url) -> Vec<(String, UrlHit)> {
        let hits: Vec<(&str, UrlHit)> = self
            .compiled
            .iter()
            .filter_map(|(id, c)| {
                c.matcher
                    .as_ref()
                    .and_then(|m| m.match_url(url))
                    .map(|h| (id.as_str(), h))
            })
            .collect();
        matching::rank(hits)
            .into_iter()
            .map(|(id, h)| (id.to_string(), h))
            .collect()
    }

    /// Stage 1 + stage 2 + schema.org fallback for a parsed page.
    pub fn plan(&self, doc: &Document) -> Plan {
        let url_hits = self.match_url(doc.url());
        // Each candidate gets its own match budget: one costly recipe's probes must not starve the others.
        let confirmed: Vec<String> = url_hits
            .iter()
            .filter(|(id, _)| {
                self.compiled[id]
                    .matcher
                    .as_ref()
                    .is_some_and(|m| m.match_page(doc, &Budget::new(MATCH_FUEL)))
            })
            .map(|(id, _)| id.clone())
            .collect();
        let detected_types = matching::detected_types(doc);
        let fallback = if confirmed.is_empty() {
            self.fallback_for(&detected_types)
        } else {
            None
        };
        let chosen = confirmed.first().cloned().or_else(|| fallback.clone());
        Plan {
            url_hits,
            confirmed,
            detected_types,
            fallback,
            chosen,
        }
    }

    /// The generic recipe for the page's main schema.org type: among detected types that map to a schema
    /// with a base recipe, the schema with the highest `fallback_rank` wins; ties keep document order
    /// (decisions.md U2, U36). A recipe page's hero VideoObject or a FAQ block thus loses to the Recipe.
    pub fn fallback_for(&self, types: &[String]) -> Option<String> {
        let mut best: Option<&(i32, String)> = None;
        for t in types {
            if let Some(hit) = self.fallback.get(t) {
                if best.is_none_or(|(rank, _)| hit.0 > *rank) {
                    best = Some(hit);
                }
            }
        }
        best.map(|(_, id)| id.clone())
    }

    pub fn extract(&self, id: &str, doc: &Document, ctx: &Ctx) -> Option<ExtractResult> {
        let c = self.compiled.get(id)?;
        Some(eval::extract(c, self.registry(), doc, ctx))
    }

    /// Matches and extracts. `None` when no recipe applies.
    pub fn extract_auto(&self, doc: &Document, ctx: &Ctx) -> (Plan, Option<ExtractResult>) {
        let plan = self.plan(doc);
        let result = plan.chosen.as_deref().and_then(|id| self.extract(id, doc, ctx));
        (plan, result)
    }

    /// Canonical protojson of an Extraction (the record `Any` expanded through the descriptor pool).
    pub fn to_json(&self, extraction: &pb::Extraction) -> serde_json::Value {
        let desc = self
            .registry()
            .pool()
            .get_message_by_name(rex_proto::EXTRACTION)
            .expect("embedded pool has Extraction");
        let mut msg = DynamicMessage::new(desc);
        msg.transcode_from(extraction)
            .expect("Extraction transcodes into its own descriptor");
        msg.serialize_with_options(serde_json::value::Serializer, &SerializeOptions::new())
            .expect("protojson serialization of an Extraction")
    }

    /// Protojson of the typed record alone.
    pub fn record_json(&self, record: &DynamicMessage) -> serde_json::Value {
        record
            .serialize_with_options(serde_json::value::Serializer, &SerializeOptions::new())
            .expect("protojson serialization of a record")
    }
}
