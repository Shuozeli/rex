//! wasm-bindgen facade over the rex engine (engine spec §11). JSON strings in, JSON strings out, so the
//! JS side stays a thin, typed wrapper.

use std::collections::BTreeMap;

use chrono::DateTime;
use prost_reflect::DescriptorPool;
use rex_dom::{Document, Limits};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinnedKeyJs {
    key_id: String,
    public: String,
}

/// Verify a bundle's `index.json` against pinned keys; returns the Index as protojson.
#[wasm_bindgen(js_name = verifyIndex)]
pub fn verify_index(index_json: &[u8], pinned_keys_json: &str) -> Result<String, JsError> {
    let keys: Vec<PinnedKeyJs> = serde_json::from_str(pinned_keys_json).map_err(js_err)?;
    let pinned = keys
        .into_iter()
        .map(|k| {
            rex_bundle::parse_hex32(&k.public)
                .map(|public| rex_bundle::PinnedKey {
                    key_id: k.key_id,
                    public,
                })
                .ok_or_else(|| JsError::new("pinned public key must be 64 hex chars"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pool = rex_proto_pool()?;
    let index = rex_bundle::verify_index(&pool, index_json, &pinned).map_err(js_err)?;
    rex_bundle::index_to_json(&pool, &index).map_err(js_err)
}

fn rex_proto_pool() -> Result<DescriptorPool, JsError> {
    SchemaRegistry::embedded().map(|r| r.pool().clone()).map_err(js_err)
}

#[wasm_bindgen(js_name = sha256Hex)]
pub fn sha256_hex(bytes: &[u8]) -> String {
    rex_bundle::sha256_hex(bytes)
}

#[wasm_bindgen(js_name = engineVersion)]
pub fn engine_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Recipe `requires` features this engine implements (JSON array). Entries needing others are skipped.
#[wasm_bindgen(js_name = supportedFeatures)]
pub fn supported_features() -> String {
    serde_json::to_string(rex_engine::recipe::SUPPORTED_FEATURES).expect("static list serializes")
}

/// Fixture redaction (decisions.md U19), identical to `rex fixture redact`.
#[wasm_bindgen(js_name = redactHtml)]
pub fn redact_html(html: &str) -> String {
    rex_redact::redact_html(html)
}

/// Redaction lint: JSON array of violations (empty when the page is clean).
#[wasm_bindgen(js_name = lintHtml)]
pub fn lint_html(html: &str) -> String {
    serde_json::to_string(&rex_redact::lint_html(html)).expect("strings serialize")
}

/// The id of one recipe (protojson), validated against the embedded schemas. Used by developer mode to
/// replace a bundle recipe with a local one.
#[wasm_bindgen(js_name = recipeId)]
pub fn recipe_id(text: &str) -> Result<String, JsError> {
    let registry = SchemaRegistry::embedded().map_err(js_err)?;
    rex_engine::recipe::parse_recipe(text, &registry)
        .map(|r| r.id)
        .map_err(js_err)
}

#[wasm_bindgen]
pub struct RexEngine {
    engine: Engine,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UrlHitJs {
    id: String,
    exact_host: bool,
    include_matched: bool,
    priority: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanJs {
    url_hits: Vec<UrlHitJs>,
    confirmed: Vec<String>,
    detected_types: Vec<String>,
    fallback: Option<String>,
    chosen: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttemptJs {
    candidate: u32,
    recipe: String,
    source: String,
    raw: Vec<String>,
    outcome: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TraceJs {
    path: String,
    attempts: Vec<AttemptJs>,
    winner: Option<u32>,
    value: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticJs {
    level: &'static str,
    code: &'static str,
    path: String,
    recipe: String,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractJs {
    plan: Option<PlanJs>,
    extraction: Option<serde_json::Value>,
    record: Option<serde_json::Value>,
    diagnostics: Vec<DiagnosticJs>,
    trace: Vec<TraceJs>,
    fuel_used: u64,
}

fn hits_js(hits: &[(String, rex_engine::UrlHit)]) -> Vec<UrlHitJs> {
    hits.iter()
        .map(|(id, h)| UrlHitJs {
            id: id.clone(),
            exact_host: h.exact_host,
            include_matched: h.include_matched,
            priority: h.priority,
        })
        .collect()
}

#[wasm_bindgen]
impl RexEngine {
    /// `fds`: the bundle's schema FileDescriptorSet. `recipes`: recipe protojson strings (bases included).
    #[wasm_bindgen(constructor)]
    pub fn new(fds: &[u8], recipes: Vec<String>) -> Result<RexEngine, JsError> {
        let pool = DescriptorPool::decode(fds).map_err(js_err)?;
        let registry = SchemaRegistry::new(pool).map_err(js_err)?;
        let mut set = RecipeSet::new(registry);
        for text in recipes {
            set.add_yaml(&text).map_err(js_err)?;
        }
        let engine = Engine::new(set, Limits::default())
            .map_err(|errs| JsError::new(&errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))?;
        Ok(RexEngine { engine })
    }

    #[wasm_bindgen(js_name = recipeIds)]
    pub fn recipe_ids(&self) -> Vec<String> {
        self.engine.recipes().ids().map(str::to_string).collect()
    }

    /// Stage 1 only (no DOM): ranked URL hits as JSON.
    #[wasm_bindgen(js_name = matchUrl)]
    pub fn match_url(&self, url: &str) -> Result<String, JsError> {
        let u = url::Url::parse(url).map_err(js_err)?;
        serde_json::to_string(&hits_js(&self.engine.match_url(&u))).map_err(js_err)
    }

    /// Parse `html`, match (unless `recipe_id` is given) and extract. Returns ExtractJs as JSON.
    pub fn extract(
        &self,
        url: &str,
        html: &str,
        recipe_id: Option<String>,
        now_ms: f64,
        hints_json: &str,
        host: &str,
    ) -> Result<String, JsError> {
        let doc = Document::parse(html, url, self.engine.limits()).map_err(js_err)?;
        let now = DateTime::from_timestamp_millis(now_ms as i64).ok_or_else(|| JsError::new("bad now_ms"))?;
        let hints: BTreeMap<String, String> = if hints_json.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_json::from_str(hints_json).map_err(js_err)?
        };
        let ctx = Ctx {
            now,
            fetch_time: Some(now),
            hints,
            source_id: String::new(),
            source_ref: String::new(),
            host: host.to_string(),
        };
        let plan = self.engine.plan(&doc);
        let chosen = recipe_id.or_else(|| plan.chosen.clone());
        let result = chosen.as_deref().and_then(|id| self.engine.extract(id, &doc, &ctx));
        let plan_js = PlanJs {
            url_hits: hits_js(&plan.url_hits),
            confirmed: plan.confirmed.clone(),
            detected_types: plan.detected_types.clone(),
            fallback: plan.fallback.clone(),
            chosen,
        };
        let out = match result {
            None => ExtractJs {
                plan: Some(plan_js),
                extraction: None,
                record: None,
                diagnostics: Vec::new(),
                trace: Vec::new(),
                fuel_used: 0,
            },
            Some(r) => ExtractJs {
                plan: Some(plan_js),
                extraction: Some(self.engine.to_json(&r.extraction)),
                record: Some(self.engine.record_json(&r.record)),
                diagnostics: r
                    .diagnostics
                    .iter()
                    .map(|d| DiagnosticJs {
                        level: match d.level {
                            rex_engine::Level::Info => "info",
                            rex_engine::Level::Warn => "warn",
                        },
                        code: d.code,
                        path: d.path.clone(),
                        recipe: d.recipe.clone(),
                        detail: d.detail.clone(),
                    })
                    .collect(),
                trace: r
                    .trace
                    .iter()
                    .map(|t| TraceJs {
                        path: t.path.clone(),
                        attempts: t
                            .attempts
                            .iter()
                            .map(|a| AttemptJs {
                                candidate: a.candidate,
                                recipe: a.recipe.clone(),
                                source: a.source.clone(),
                                raw: a.raw.clone(),
                                outcome: a.outcome.clone(),
                            })
                            .collect(),
                        winner: t.winner,
                        value: t.value.clone(),
                    })
                    .collect(),
                fuel_used: r.fuel_used,
            },
        };
        serde_json::to_string(&out).map_err(js_err)
    }
}
