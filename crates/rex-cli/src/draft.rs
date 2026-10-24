//! `rex draft` (design.md §7.2, FR-RC-7): BYO-LLM recipe drafting.
//!
//! Builds an evidence pack from one page (DOM skeleton, JSON-LD, meta tags, repeating structures, the
//! target schema with its schema.org bindings, an example recipe for the same schema), asks a
//! user-configured endpoint (OpenAI-compatible chat completions or Gemini generateContent, temperature 0)
//! for one `ExtractionRecipe`, parses it strictly, and retries at most twice, for structural errors only
//! (unparseable, does not compile). Extraction quality is reported, never iterated on: the draft lands in
//! `recipes/drafts/` and goes through `rex test` and review like any hand-written recipe.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use rex_dom::{Document, Limits};
use rex_engine::recipe::RecipeProto;
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::{RecordSpec, SchemaRegistry, StringKind, Target};

use crate::gate;
use scraper::{ElementRef, Html, Node, Selector};

pub const RECIPE_PROTO: &str = include_str!("../../../proto/rex/extract/v1/extraction_recipe.proto");
const MAX_ATTEMPTS: usize = 3;
const SKELETON_BUDGET: usize = 24_000;
/// Budget for the whole-page skeleton when the focused section is present.
const SKELETON_BUDGET_FOCUSED: usize = 12_000;
const FOCUS_BUDGET: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub struct Turn {
    pub role: Role,
    pub text: String,
}

/// A chat model. `complete` returns the assistant's text for the conversation so far.
pub trait Llm {
    fn describe(&self) -> String;
    fn complete(&self, system: &str, turns: &[Turn]) -> Result<String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Api {
    /// POST {endpoint} with an OpenAI chat-completions body (the endpoint is the full .../chat/completions URL).
    OpenAi,
    /// POST {endpoint} with a Gemini generateContent body (the endpoint is the full ...:generateContent URL).
    Gemini,
}

impl Api {
    pub fn detect(endpoint: &str) -> Api {
        if endpoint.contains(":generateContent") {
            Api::Gemini
        } else {
            Api::OpenAi
        }
    }
}

pub struct HttpLlm {
    agent: ureq::Agent,
    endpoint: String,
    api: Api,
    model: String,
    api_key: Option<String>,
    /// Send the system prompt as the head of the first user turn (for endpoints without system roles).
    inline_system: bool,
}

impl HttpLlm {
    pub fn new(endpoint: &str, api: Api, model: &str, api_key: Option<String>, inline_system: bool) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(600)))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            agent,
            endpoint: endpoint.to_string(),
            api,
            model: model.to_string(),
            api_key,
            inline_system,
        }
    }
}

#[derive(serde::Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(serde::Serialize)]
struct OpenAiFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(serde::Serialize)]
struct OpenAiRequest<'a> {
    model: &'a str,
    temperature: f32,
    messages: Vec<OpenAiMessage<'a>>,
    response_format: OpenAiFormat,
}

#[derive(serde::Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(serde::Deserialize)]
struct OpenAiChoice {
    message: OpenAiContent,
}

#[derive(serde::Deserialize)]
struct OpenAiContent {
    content: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GeminiPart {
    text: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiConfig {
    temperature: f32,
    response_mime_type: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    contents: Vec<GeminiContent>,
    generation_config: GeminiConfig,
}

#[derive(serde::Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(serde::Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}

impl Llm for HttpLlm {
    /// API kind and model only: endpoint URLs can be private and drafts are committed.
    fn describe(&self) -> String {
        match self.api {
            Api::OpenAi => format!("an OpenAI-compatible model ({})", self.model),
            Api::Gemini => {
                let model = self
                    .endpoint
                    .rsplit("/models/")
                    .next()
                    .and_then(|m| m.split(':').next())
                    .filter(|m| !m.is_empty() && !m.contains('/'))
                    .unwrap_or("unknown");
                format!("Gemini ({model})")
            }
        }
    }

    fn complete(&self, system: &str, turns: &[Turn]) -> Result<String> {
        self.complete_inner(system, turns)
            .map_err(|e| anyhow!("{}", self.scrub(&format!("{e:#}"))))
    }
}

impl HttpLlm {
    /// `text` with the endpoint URL, its host and the API key replaced by placeholders.
    fn scrub(&self, text: &str) -> String {
        let mut out = text.replace(&self.endpoint, "<llm-endpoint>");
        if let Some(host) = url::Url::parse(&self.endpoint)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
        {
            out = out.replace(&host, "<llm-host>");
        }
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            out = out.replace(key, "<api-key>");
        }
        out
    }

    fn complete_inner(&self, system: &str, turns: &[Turn]) -> Result<String> {
        let inlined: Vec<Turn>;
        let (system, turns) = if self.inline_system {
            inlined = vec![Turn {
                role: Role::User,
                text: flatten(system, turns),
            }];
            ("", inlined.as_slice())
        } else {
            (system, turns)
        };
        let body = match self.api {
            Api::OpenAi => {
                let mut messages = Vec::new();
                if !system.is_empty() {
                    messages.push(OpenAiMessage {
                        role: "system",
                        content: system,
                    });
                }
                messages.extend(turns.iter().map(|t| OpenAiMessage {
                    role: match t.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    },
                    content: &t.text,
                }));
                serde_json::to_string(&OpenAiRequest {
                    model: &self.model,
                    temperature: 0.0,
                    messages,
                    response_format: OpenAiFormat { kind: "json_object" },
                })?
            }
            Api::Gemini => serde_json::to_string(&GeminiRequest {
                system_instruction: (!system.is_empty()).then(|| GeminiContent {
                    role: None,
                    parts: vec![GeminiPart {
                        text: system.to_string(),
                    }],
                }),
                contents: turns
                    .iter()
                    .map(|t| GeminiContent {
                        role: Some(
                            match t.role {
                                Role::User => "user",
                                Role::Assistant => "model",
                            }
                            .to_string(),
                        ),
                        parts: vec![GeminiPart { text: t.text.clone() }],
                    })
                    .collect(),
                generation_config: GeminiConfig {
                    temperature: 0.0,
                    response_mime_type: "application/json",
                },
            })?,
        };
        let mut req = self
            .agent
            .post(&self.endpoint)
            .header("content-type", "application/json");
        if let Some(key) = &self.api_key {
            req = match self.api {
                Api::OpenAi => req.header("authorization", &format!("Bearer {key}")),
                Api::Gemini => req.header("x-goog-api-key", key),
            };
        }
        // The endpoint can be a private host (and some put the key in the URL): errors name it generically.
        let mut resp = req
            .send(body.as_bytes())
            .map_err(|e| anyhow!("POST to the LLM endpoint failed: {}", self.scrub(&e.to_string())))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().with_config().limit(16 << 20).read_to_string()?;
        if status != 200 {
            bail!(
                "LLM endpoint returned HTTP {status}: {}",
                text.chars().take(400).collect::<String>()
            );
        }
        match self.api {
            Api::OpenAi => {
                let r: OpenAiResponse = serde_json::from_str(&text).context("chat-completions response")?;
                r.choices
                    .into_iter()
                    .next()
                    .and_then(|c| c.message.content)
                    .ok_or_else(|| anyhow!("chat-completions response has no content"))
            }
            Api::Gemini => {
                let r: GeminiResponse = serde_json::from_str(&text).context("generateContent response")?;
                let out: String = r
                    .candidates
                    .into_iter()
                    .next()
                    .and_then(|c| c.content)
                    .map(|c| c.parts.into_iter().map(|p| p.text).collect())
                    .unwrap_or_default();
                if out.is_empty() {
                    bail!(
                        "generateContent response has no text: {}",
                        text.chars().take(300).collect::<String>()
                    );
                }
                Ok(out)
            }
        }
    }
}

/// One user message holding the system prompt and the whole conversation so far, for endpoints and proxies
/// that mishandle system roles or multi-turn requests (`--inline-system`).
fn flatten(system: &str, turns: &[Turn]) -> String {
    let mut text = format!("{system}\n\n---\n\n");
    for (i, t) in turns.iter().enumerate() {
        match t.role {
            Role::User if i == 0 => text.push_str(&t.text),
            Role::User => {
                text.push_str("\n\n---\n\n");
                text.push_str(&t.text);
            }
            Role::Assistant => {
                text.push_str("\n\n---\n\nYour previous answer was:\n");
                text.push_str(&t.text);
            }
        }
    }
    text
}

// ---- evidence pack ---------------------------------------------------------------------------------

fn short(s: &str, n: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= n {
        s
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn kind_name(t: &Target) -> String {
    match t {
        Target::Str(k) => match k {
            StringKind::String => "string",
            StringKind::Text => "text (multi-paragraph)",
            StringKind::Html => "html (sanitized)",
            StringKind::Url => "url (absolute)",
            StringKind::Decimal => "decimal",
            StringKind::Date => "date (YYYY-MM-DD)",
        }
        .to_string(),
        Target::I32 | Target::I64 | Target::U32 | Target::U64 => "integer".into(),
        Target::F32 | Target::F64 => "number".into(),
        Target::Bool => "bool".into(),
        Target::Timestamp => "timestamp (any date/time text; parsed)".into(),
        Target::Enum(e) => format!(
            "enum {} [{}]",
            e.name(),
            e.values().map(|v| v.name().to_string()).collect::<Vec<_>>().join(", ")
        ),
        Target::Message(m) => format!("record {}", m.full_name()),
    }
}

fn describe_record(registry: &SchemaRegistry, spec: &RecordSpec, indent: usize, out: &mut String) -> Result<()> {
    let pad = "  ".repeat(indent);
    for f in &spec.fields {
        let mut notes = Vec::new();
        if f.required {
            notes.push("required".to_string());
        }
        if f.repeated {
            notes.push("repeated".to_string());
        }
        if !f.schema_org.is_empty() {
            notes.push(format!("schema.org {}", f.schema_org));
        }
        out.push_str(&format!(
            "{pad}- {}: {}{}\n",
            f.name,
            kind_name(&f.target),
            if notes.is_empty() {
                String::new()
            } else {
                format!(" ({})", notes.join(", "))
            }
        ));
        if let Target::Message(m) = &f.target {
            describe_record(registry, &registry.record_of(m)?, indent + 1, out)?;
        }
    }
    Ok(())
}

fn signature(e: &ElementRef) -> String {
    let v = e.value();
    let mut s = v.name().to_string();
    if let Some(id) = v.id() {
        s.push('#');
        s.push_str(id);
    }
    let mut classes: Vec<&str> = v.classes().collect();
    classes.sort_unstable();
    for c in classes.iter().take(4) {
        s.push('.');
        s.push_str(c);
    }
    for a in [
        "itemprop",
        "itemtype",
        "datetime",
        "role",
        "rel",
        "property",
        "data-testid",
        "data-component",
    ] {
        if let Some(val) = v.attr(a) {
            s.push_str(&format!("[{a}={}]", short(val, 40)));
        }
    }
    s
}

/// Text directly inside the element (not in children).
fn own_text(e: &ElementRef) -> String {
    let mut t = String::new();
    for c in e.children() {
        if let Node::Text(x) = c.value() {
            t.push_str(x);
            t.push(' ');
        }
    }
    short(&t, 70)
}

const SKIP: &[&str] = &[
    "head", "script", "style", "noscript", "svg", "iframe", "template", "link", "meta", "path", "br",
];

fn skeleton(e: ElementRef, depth: usize, out: &mut String, truncated: &mut bool) {
    skeleton_within(e, depth, out, truncated, SKELETON_BUDGET);
}

fn skeleton_within(e: ElementRef, depth: usize, out: &mut String, truncated: &mut bool, budget: usize) {
    if out.len() > budget {
        *truncated = true;
        return;
    }
    let kids: Vec<ElementRef> = e
        .children()
        .filter_map(ElementRef::wrap)
        .filter(|k| !SKIP.contains(&k.value().name()))
        .collect();
    let text = own_text(&e);
    out.push_str(&"  ".repeat(depth));
    out.push_str(&signature(&e));
    if !text.is_empty() {
        out.push_str(&format!(" \"{text}\""));
    }
    out.push('\n');
    let mut i = 0;
    while i < kids.len() {
        let sig = signature(&kids[i]);
        let mut run = 1;
        while i + run < kids.len() && signature(&kids[i + run]) == sig {
            run += 1;
        }
        // Show the first two of a run of identical siblings in full, then summarize.
        for k in kids.iter().skip(i).take(run.min(2)) {
            skeleton_within(*k, depth + 1, out, truncated, budget);
        }
        if run > 2 {
            out.push_str(&format!("{}… ×{} more {sig}\n", "  ".repeat(depth + 1), run - 2));
        }
        i += run;
    }
}

/// `body > div#app > main.content > article` for an element (signatures of its ancestors).
fn ancestor_path(e: &ElementRef) -> String {
    let mut chain: Vec<String> = e
        .ancestors()
        .filter_map(ElementRef::wrap)
        .take_while(|a| a.value().name() != "html")
        .map(|a| signature(&a))
        .collect();
    chain.reverse();
    chain.push(signature(e));
    chain.join(" > ")
}

/// The parts of the page a recipe needs, found structurally: the readability scorer's main container (its
/// ancestor path and subtree), the <h1>s, and elements that look like a byline or date.
fn focused(html: &str, url: &str) -> String {
    let limits = Limits::default();
    let Ok(doc) = Document::parse(html, url, &limits) else {
        return String::new();
    };
    let budget = rex_dom::Budget::new(limits.fuel);
    let mut out = String::new();
    if let Ok(Some(main)) = rex_readability::main_container(&doc, &budget) {
        out.push_str(&format!("Main content container: {}\n", ancestor_path(&main)));
        let mut sub = String::new();
        let mut truncated = false;
        skeleton_within(main, 1, &mut sub, &mut truncated, FOCUS_BUDGET);
        out.push_str(&sub);
        if truncated {
            out.push_str("  (container skeleton truncated)\n");
        }
    }
    let h1 = Selector::parse("h1").expect("static selector");
    for h in doc.root().select(&h1).take(3) {
        out.push_str(&format!(
            "Headline: {} \"{}\"\n",
            ancestor_path(&h),
            short(&h.text().collect::<String>(), 100)
        ));
    }
    let meta_sel = Selector::parse(
        "time, [datetime], [itemprop~=author], [itemprop~=datePublished], [rel~=author], [class*=author], \
         [class*=byline], [class*=date], [class*=publish], [class*=time]",
    )
    .expect("static selector");
    let mut seen = 0;
    for e in doc.root().select(&meta_sel) {
        let text = short(&e.text().collect::<String>(), 60);
        if text.is_empty() && e.value().attr("datetime").is_none() {
            continue;
        }
        out.push_str(&format!("Byline/date candidate: {} \"{text}\"\n", ancestor_path(&e)));
        seen += 1;
        if seen == 12 {
            break;
        }
    }
    out
}

/// Sibling groups of >= 4 elements with the same tag and classes (list items, cards, rows).
fn repeating(html: &Html) -> Vec<String> {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    let all = Selector::parse("*").expect("static selector");
    for parent in html.select(&all) {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for k in parent.children().filter_map(ElementRef::wrap) {
            let v = k.value();
            if SKIP.contains(&v.name()) || v.classes().next().is_none() {
                continue;
            }
            let mut classes: Vec<&str> = v.classes().collect();
            classes.sort_unstable();
            *counts
                .entry(format!("{} > {}.{}", signature(&parent), v.name(), classes.join(".")))
                .or_default() += 1;
        }
        for (k, n) in counts {
            if n >= 4 {
                let e = groups.entry(k).or_default();
                *e = (*e).max(n);
            }
        }
    }
    let mut v: Vec<(String, usize)> = groups.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v.into_iter().take(15).map(|(k, n)| format!("- {k} ×{n}")).collect()
}

pub struct PackInput<'a> {
    pub url: &'a str,
    pub html: &'a str,
    pub schema: &'a RecordSpec,
    pub registry: &'a SchemaRegistry,
    pub id: &'a str,
    pub base: Option<&'a str>,
    /// (id, canonical JSON) of an existing recipe for the same schema.
    pub example: Option<(&'a str, &'a str)>,
    /// URL rules fixed by the caller (harvesting); the model then writes only page conditions and fields.
    pub urls: Option<&'a UrlRules>,
}

/// The user message: everything the model may use, and nothing else.
pub fn evidence_pack(p: &PackInput) -> Result<String> {
    let html = Html::parse_document(p.html);
    let mut out = String::new();
    let host = url::Url::parse(p.url)?
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_string();
    out.push_str(&format!(
        "## Task\nWrite the recipe `{}` (version 1) for schema `{}` for pages like {}.\n",
        p.id,
        p.schema.full_name(),
        p.url
    ));
    match p.base {
        Some(base) => out.push_str(&format!(
            "Set `extends: \"{base}\"`. The base already reads JSON-LD, Open Graph, meta tags and a readability \
             fallback for every field, so add a field rule only where a site CSS selector is more precise (typically \
             the title, the body, authors or dates); omit fields the base covers well.\n"
        )),
        None => out.push_str("There is no base recipe for this schema: give every required field a rule.\n"),
    }
    match p.urls {
        Some(rules) => out.push_str(&format!(
            "`match.urls` is fixed by the tool as hosts {:?}, include {:?}: do not write it. Add a `match.page` \
             condition (a JSON-LD type, a meta value or a CSS selector that every page of this kind has and other \
             pages lack).\n\n",
            rules.hosts, rules.include
        )),
        None => out.push_str(&format!(
            "`match.urls.hosts` must be [\"{host}\"]; add an `include` RE2 path regex for this kind of page, and a \
             `match.page` condition (a JSON-LD type, a meta value or a CSS selector that only these pages have).\n\n"
        )),
    }

    out.push_str(&format!(
        "## Target schema {} (schema.org {})\n",
        p.schema.full_name(),
        p.schema.schema_org_types().join(", ")
    ));
    describe_record(p.registry, p.schema, 0, &mut out)?;

    let ld = Selector::parse(r#"script[type="application/ld+json"]"#).expect("static selector");
    let blocks: Vec<String> = html.select(&ld).map(|s| s.text().collect::<String>()).collect();
    out.push_str(&format!("\n## JSON-LD ({} blocks)\n", blocks.len()));
    for b in blocks.iter().take(4) {
        let pretty = serde_json::from_str::<serde_json::Value>(b)
            .ok()
            .and_then(|v| serde_json::to_string(&v).ok())
            .unwrap_or_else(|| b.trim().to_string());
        out.push_str(&short(&pretty, 1800));
        out.push('\n');
    }

    let meta = Selector::parse("meta[content]").expect("static selector");
    out.push_str("\n## Meta tags\n");
    for m in html.select(&meta).take(50) {
        let v = m.value();
        if let Some(k) = v
            .attr("property")
            .or_else(|| v.attr("name"))
            .or_else(|| v.attr("itemprop"))
        {
            out.push_str(&format!(
                "- {k} = {}\n",
                short(v.attr("content").unwrap_or_default(), 120)
            ));
        }
    }

    out.push_str("\n## Repeating structures (parent > child ×count)\n");
    for line in repeating(&html) {
        out.push_str(&line);
        out.push('\n');
    }

    let focus = focused(p.html, p.url);
    if !focus.is_empty() {
        out.push_str("\n## Where the content is (main container picked by a readability scorer, headlines, byline/date candidates)\n");
        out.push_str(&focus);
    }
    let body = Selector::parse("body").expect("static selector");
    out.push_str("\n## DOM skeleton (tag#id.classes[attrs] \"own text\"; runs of identical siblings collapsed)\n");
    if let Some(b) = html.select(&body).next() {
        let mut truncated = false;
        let mut sk = String::new();
        let budget = if focus.is_empty() {
            SKELETON_BUDGET
        } else {
            SKELETON_BUDGET_FOCUSED
        };
        skeleton_within(b, 0, &mut sk, &mut truncated, budget);
        out.push_str(&sk);
        if truncated {
            out.push_str("(skeleton truncated)\n");
        }
    }

    if let Some((id, json)) = p.example {
        out.push_str(&format!(
            "\n## Example: the reviewed recipe `{id}` for the same schema\n{json}\n"
        ));
    }
    out.push_str("\nReturn only the JSON object.\n");
    Ok(out)
}

pub fn system_prompt() -> String {
    format!(
        "You write extraction recipes for rex, a declarative web-page-to-record extractor. A recipe is one \
         `rex.extract.v1.ExtractionRecipe` message in protobuf JSON (protojson) form; field names may be \
         lowerCamelCase or snake_case, enums are their full names (e.g. \"TAKE_HTML\").\n\n\
         Rules:\n\
         - Output exactly one JSON object and nothing else.\n\
         - Use only selectors, JSON-LD keys and meta keys that appear in the evidence. Prefer stable ids, \
           semantic tags, itemprop and data attributes over presentational class names.\n\
         - Each field rule lists candidates in priority order; the first candidate with a value wins.\n\
         - Mark `expect: true` on a field whose site selector should always hit on these pages, so that site \
           changes are detected.\n\
         - For repeated records (lists of items), use `each` with a selector for the item and fields relative to it.\n\
         - Do not invent features: only messages and fields defined below exist.\n\n\
         The recipe format (proto3, with comments):\n\n{RECIPE_PROTO}"
    )
}

/// The JSON object inside a reply (tolerates code fences and surrounding prose).
pub fn extract_json(reply: &str) -> Result<&str> {
    let none = || anyhow!("no JSON object in the reply: {:?}", short(reply, 200));
    let start = reply.find('{').ok_or_else(none)?;
    let end = reply.rfind('}').ok_or_else(none)?;
    if end < start {
        return Err(none());
    }
    Ok(&reply[start..=end])
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Protojson uses lowerCamelCase keys; hand-written recipes use the proto (snake_case) names. Recipes have no
/// map fields, so every object key is a field name.
fn snake_keys(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(m) => serde_json::Value::Object(
            m.into_iter()
                .map(|(k, v)| (camel_to_snake(&k), snake_keys(v)))
                .collect(),
        ),
        serde_json::Value::Array(a) => serde_json::Value::Array(a.into_iter().map(snake_keys).collect()),
        other => other,
    }
}

pub struct DraftRequest<'a> {
    pub id: &'a str,
    pub schema: &'a str,
    pub url: &'a str,
    pub html: &'a str,
    pub source_label: &'a str,
    pub now: DateTime<Utc>,
    /// More pages of the same kind; the quality gate runs on them too.
    pub also: &'a [gate::Page],
    /// Pages the recipe must not match (e.g. the homepage for an article recipe).
    pub negatives: &'a [gate::Page],
    /// Quality-gate repair rounds (decisions.md U31); 0 = report problems without asking again.
    pub repair_rounds: usize,
    /// URL rules computed from real URLs (harvesting). When set they replace the model's `match.urls`, and the
    /// prompt says so: the model only writes page conditions and fields.
    pub urls: Option<UrlRules>,
    /// Ids that may serve as the prompt's base and example recipe (reviewed ones, not other drafts or auto
    /// recipes); `None` allows every recipe in the set.
    pub reviewed: Option<&'a BTreeSet<String>>,
}

#[derive(Clone, Debug)]
pub struct UrlRules {
    pub hosts: Vec<String>,
    pub include: Vec<String>,
}

#[derive(Debug)]
pub struct Draft {
    pub yaml: String,
    pub attempts: usize,
    pub repairs: usize,
    pub confirmed: bool,
    pub status: String,
    pub filled: Vec<String>,
    pub missing_required: Vec<String>,
    /// Quality-gate problems left after the repair rounds; empty = passed.
    pub problems: Vec<String>,
}

/// The evidence pack for `req`, with the schema's base recipe and a reviewed example taken from `set`.
pub fn pack_for(set: &RecipeSet, req: &DraftRequest) -> Result<String> {
    let registry = set.registry();
    let spec = registry.record(req.schema)?;
    let mut base = None;
    let mut example = None;
    for id in set.ids() {
        let r = set.get(id).expect("listed id exists");
        if r.schema != req.schema || req.reviewed.is_some_and(|ok| !ok.contains(id)) {
            continue;
        }
        if rex_engine::recipe::is_base_recipe(r) {
            base.get_or_insert_with(|| r.id.clone());
        } else if example.is_none() {
            let (json, _) = rex_engine::recipe::canonical_json(r, registry).map_err(|e| anyhow!(e))?;
            example = Some((r.id.clone(), json));
        }
    }
    evidence_pack(&PackInput {
        url: req.url,
        html: req.html,
        schema: &spec,
        registry,
        id: req.id,
        base: base.as_deref(),
        example: example.as_ref().map(|(i, j)| (i.as_str(), j.as_str())),
        urls: req.urls.as_ref(),
    })
}

/// Draft a recipe. `set` holds the existing recipes (bases and examples come from it).
pub fn draft(llm: &dyn Llm, set: &RecipeSet, req: &DraftRequest) -> Result<Draft> {
    if set.ids().any(|i| i == req.id) {
        bail!("recipe `{}` already exists; choose another --id", req.id);
    }
    let pack = pack_for(set, req)?;
    let system = system_prompt();
    let limits = Limits::default();
    let doc = Document::parse(req.html, req.url, &limits)?;
    let mut turns = vec![Turn {
        role: Role::User,
        text: pack,
    }];
    let mut positives = vec![gate::Page {
        label: "page 1".into(),
        url: req.url.to_string(),
        html: req.html.to_string(),
    }];
    positives.extend(req.also.iter().map(|p| gate::Page {
        label: p.label.clone(),
        url: p.url.clone(),
        html: p.html.clone(),
    }));
    let mut attempt = 0;
    let mut repairs = 0;
    // The last recipe that compiled; kept so that a repair round whose replies never compile again still
    // returns it (with its gate problems) instead of nothing.
    let mut last_compiled: Option<(Engine, RecipeProto, gate::Report)> = None;
    loop {
        attempt += 1;
        let reply = llm.complete(&system, &turns)?;
        match compile_reply(set, req, &reply, &limits) {
            Ok((engine, recipe)) => {
                let report = gate::check(&engine, req.id, req.schema, &positives, req.negatives, req.now);
                // Too few full pages is a property of the sample, not of the recipe: asking again cannot fix it.
                if !report.pass && report.recipe_fixable() && repairs < req.repair_rounds {
                    repairs += 1;
                    eprintln!("repair round {repairs}: {}", report.problems().join("; "));
                    turns.push(Turn {
                        role: Role::Assistant,
                        text: reply,
                    });
                    turns.push(Turn {
                        role: Role::User,
                        text: repair_prompt(&report, &positives),
                    });
                    last_compiled = Some((engine, recipe, report));
                    continue;
                }
                return finish(llm, req, &doc, &engine, &recipe, &report, attempt, repairs);
            }
            Err(e) => {
                eprintln!("attempt {attempt}: {e:#}");
                if attempt - repairs >= MAX_ATTEMPTS {
                    if let Some((engine, recipe, report)) = &last_compiled {
                        eprintln!("keeping the last recipe that compiled (it fails the quality gate)");
                        return finish(llm, req, &doc, engine, recipe, report, attempt, repairs);
                    }
                    bail!("no valid recipe after {MAX_ATTEMPTS} attempts: {e:#}");
                }
                turns.push(Turn {
                    role: Role::Assistant,
                    text: reply,
                });
                turns.push(Turn {
                    role: Role::User,
                    text: format!(
                        "That recipe is invalid: {e:#}\nReturn the corrected recipe as one JSON object, changing only \
                         what the error requires."
                    ),
                });
            }
        }
    }
}

/// The draft for a compiled recipe: its YAML, and how it did on the first page.
#[allow(clippy::too_many_arguments)]
fn finish(
    llm: &dyn Llm,
    req: &DraftRequest,
    doc: &Document,
    engine: &Engine,
    recipe: &RecipeProto,
    report: &gate::Report,
    attempts: usize,
    repairs: usize,
) -> Result<Draft> {
    let registry = engine.recipes().registry();
    let ctx = Ctx {
        now: req.now,
        fetch_time: Some(req.now),
        hints: BTreeMap::new(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "rex-draft".into(),
    };
    let confirmed = engine.plan(doc).confirmed.iter().any(|c| c == req.id);
    let result = engine
        .extract(req.id, doc, &ctx)
        .ok_or_else(|| anyhow!("the draft did not run on the page"))?;
    let ext = engine.to_json(&result.extraction);
    let record = engine.record_json(&result.record);
    let filled = record
        .as_object()
        .map(|o| o.keys().filter(|k| *k != "@type").map(|k| camel_to_snake(k)).collect())
        .unwrap_or_default();
    let strings = |k: &str| -> Vec<String> {
        ext.get(k)
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    let (json, _) = rex_engine::recipe::canonical_json(recipe, registry).map_err(|e| anyhow!(e))?;
    let value = snake_keys(serde_json::from_str(&json)?);
    let header = format!(
        "# Drafted by `rex draft` with {} on {} from {}.\n\
         # Unreviewed: add >= 2 redacted fixtures, run `rex test`, review the selectors, then move it to\n\
         # recipes/sites/ in a PR.\n",
        llm.describe(),
        req.now.format("%Y-%m-%d"),
        req.source_label
    );
    Ok(Draft {
        yaml: header + &serde_yaml::to_string(&value)?,
        attempts,
        repairs,
        problems: report.problems(),
        confirmed,
        status: ext
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("?")
            .trim_start_matches("EXTRACTION_STATUS_")
            .to_string(),
        filled,
        missing_required: strings("missingRequired"),
    })
}

/// Feedback for a recipe that compiles but fails the quality gate: the problems, plus the skeleton of the
/// first failing page the model has not seen yet.
fn repair_prompt(report: &gate::Report, positives: &[gate::Page]) -> String {
    let mut text = format!(
        "The recipe compiles but fails quality checks on real pages of this site:\n- {}\n",
        report.problems().join("\n- ")
    );
    let unseen = report
        .pages
        .iter()
        .filter(|o| !o.problems.is_empty() && o.label != "page 1")
        .find_map(|o| positives.iter().find(|p| p.label == o.label));
    if let Some(p) = unseen {
        text.push_str(&format!(
            "\nDOM skeleton of {} ({}):\n{}\n",
            p.label,
            p.url,
            page_skeleton(&p.html)
        ));
    }
    text.push_str(
        "\nFix the selectors and match rules so the recipe works on all of these pages (prefer structure that \
         is shared across pages over page-specific classes). Return the whole corrected recipe as one JSON object.",
    );
    text
}

fn page_skeleton(html: &str) -> String {
    let doc = Html::parse_document(html);
    let body = Selector::parse("body").expect("static selector");
    let mut out = String::new();
    let mut truncated = false;
    if let Some(b) = doc.select(&body).next() {
        skeleton(b, 0, &mut out, &mut truncated);
    }
    if truncated {
        out.push_str("(skeleton truncated)\n");
    }
    out
}

fn compile_reply(
    set: &RecipeSet,
    req: &DraftRequest,
    reply: &str,
    limits: &Limits,
) -> Result<(Engine, rex_engine::recipe::RecipeProto)> {
    let registry = set.registry();
    let json = extract_json(reply)?;
    let mut recipe = rex_engine::recipe::parse_recipe(json, registry).map_err(|e| anyhow!("{e}"))?;
    if let Some(rules) = &req.urls {
        let m = recipe.r#match.get_or_insert_with(Default::default);
        m.urls = Some(rex_proto::rex::extract::v1::UrlMatch {
            hosts: rules.hosts.clone(),
            include: rules.include.clone(),
            exclude: Vec::new(),
        });
    }
    if recipe
        .r#match
        .as_ref()
        .and_then(|m| m.urls.as_ref())
        .is_none_or(|u| u.hosts.is_empty())
    {
        bail!("a site recipe needs match.urls.hosts");
    }
    if recipe.fields.is_empty() {
        bail!("the recipe has no field rules");
    }
    // These are not the model's to choose.
    recipe.id = req.id.to_string();
    recipe.version = 1;
    recipe.schema = req.schema.to_string();
    let (canonical, _) = rex_engine::recipe::canonical_json(&recipe, registry).map_err(|e| anyhow!(e))?;
    let mut with = set.clone();
    with.add_yaml(&canonical).map_err(|e| anyhow!("{e}"))?;
    let engine = Engine::new(with, limits.clone())
        .map_err(|errs| anyhow!(errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")))?;
    Ok((engine, recipe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Replays scripted replies and records what it was sent.
    struct FakeLlm {
        replies: RefCell<Vec<String>>,
        seen: RefCell<Vec<usize>>,
    }

    impl FakeLlm {
        fn new(replies: &[&str]) -> Self {
            Self {
                replies: RefCell::new(replies.iter().rev().map(|s| s.to_string()).collect()),
                seen: RefCell::new(Vec::new()),
            }
        }
    }

    impl Llm for FakeLlm {
        fn describe(&self) -> String {
            "fake".into()
        }
        fn complete(&self, _system: &str, turns: &[Turn]) -> Result<String> {
            self.seen.borrow_mut().push(turns.len());
            self.replies
                .borrow_mut()
                .pop()
                .ok_or_else(|| anyhow!("no more replies"))
        }
    }

    const BASE: &str = "id: generic/article.v1\nversion: 1\nschema: rex.schema.article.v1.Article\nfields:\n  - path: title\n    candidates: [ { json_ld: {} }, { meta: { key: \"og:title\" } } ]\n";
    const PAGE: &str = r#"<html><head><title>T</title><meta property="og:title" content="Hello"></head><body><main id="post"><h1 class="post-title">Hello world</h1><div class="post-body"><p>One two three.</p></div></main></body></html>"#;
    const GOOD: &str = r#"{"id":"x","version":7,"schema":"y","extends":"generic/article.v1","match":{"urls":{"hosts":["blog.example"]},"page":{"any":[{"exists":"main#post"}]}},"fields":[{"path":"title","expect":true,"candidates":[{"css":{"selector":"h1.post-title"}}]}]}"#;

    fn set() -> RecipeSet {
        let mut s = RecipeSet::new(SchemaRegistry::embedded().unwrap());
        s.add_yaml(BASE).unwrap();
        s
    }

    fn request() -> DraftRequest<'static> {
        DraftRequest {
            id: "blog.example/post",
            schema: "rex.schema.article.v1.Article",
            url: "https://blog.example/2026/hello",
            html: PAGE,
            source_label: "test",
            now: DateTime::parse_from_rfc3339("2026-10-13T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            also: &[],
            negatives: &[],
            repair_rounds: 0,
            urls: None,
            reviewed: None,
        }
    }

    #[test]
    fn valid_reply_becomes_a_snake_case_yaml_draft_with_forced_identity() {
        // Arrange
        let llm = FakeLlm::new(&[&format!("```json\n{GOOD}\n```")]);

        // Act
        let d = draft(&llm, &set(), &request()).unwrap();

        // Assert
        assert_eq!(d.attempts, 1);
        assert!(d.confirmed);
        assert!(d.yaml.contains("id: blog.example/post"), "{}", d.yaml);
        assert!(d.yaml.contains("version: 1"), "{}", d.yaml);
        assert!(d.yaml.contains("schema: rex.schema.article.v1.Article"), "{}", d.yaml);
        assert!(d.filled.contains(&"title".to_string()), "{:?}", d.filled);
    }

    #[test]
    fn structural_error_is_retried_with_the_error_and_then_succeeds() {
        // Arrange
        let llm = FakeLlm::new(&["{\"fields\": [{\"path\": \"no_such_field\"}]}", GOOD]);

        // Act
        let d = draft(&llm, &set(), &request()).unwrap();

        // Assert
        assert_eq!(d.attempts, 2);
        assert_eq!(
            *llm.seen.borrow(),
            vec![1, 3],
            "the retry carries the bad reply and the error"
        );
    }

    #[test]
    fn gives_up_after_three_structural_failures() {
        // Arrange
        let llm = FakeLlm::new(&["not json", "still not", "{}{"]);

        // Act
        let r = draft(&llm, &set(), &request());

        // Assert
        assert!(r.unwrap_err().to_string().contains("after 3 attempts"));
    }

    fn long_page(title: &str) -> String {
        let body = "A sentence about declarative extraction recipes and typed records. ".repeat(8);
        format!(
            r#"<html><head><title>{title} - Example Blog</title><meta property="og:title" content="{title}"></head><body><main id="post"><h1 class="post-title">{title}</h1><div class="post-body"><p>{body}</p><p>{body}</p></div></main></body></html>"#
        )
    }

    #[test]
    fn gate_failure_is_sent_back_and_repaired() {
        // Arrange
        let bad = GOOD.replace("h1.post-title", "h2.headline");
        let good = format!(
            "{}{}",
            &GOOD[..GOOD.len() - 2],
            r#",{"path":"body_html","candidates":[{"css":{"selector":"div.post-body","take":"TAKE_HTML"}}]},{"path":"body_text","candidates":[{"field":{"path":"body_html"}}]}]}"#
        );
        let llm = FakeLlm::new(&[&bad, &good]);
        let page1 = long_page("Hello world");
        let also = [gate::Page {
            label: "page 2".into(),
            url: "https://blog.example/2026/second".into(),
            html: long_page("Second post"),
        }];
        let mut req = request();
        req.html = &page1;
        req.also = &also;
        req.repair_rounds = 2;

        // Act
        let d = draft(&llm, &set(), &req).unwrap();

        // Assert
        assert_eq!(d.repairs, 1);
        assert!(d.problems.is_empty(), "{:?}", d.problems);
        assert_eq!(
            *llm.seen.borrow(),
            vec![1, 3],
            "the repair turn carries the bad reply and the gate problems"
        );
    }

    #[test]
    fn gate_problems_are_reported_when_repairs_are_off() {
        // Arrange
        let llm = FakeLlm::new(&[GOOD]);

        // Act
        let d = draft(&llm, &set(), &request()).unwrap();

        // Assert
        assert_eq!(d.repairs, 0);
        assert!(
            d.problems.iter().any(|p| p.contains("enough readable text")),
            "{:?}",
            d.problems
        );
    }

    #[test]
    fn too_few_full_pages_does_not_spend_repair_rounds() {
        // Arrange
        let llm = FakeLlm::new(&[GOOD]);
        let mut req = request();
        req.repair_rounds = 2;

        // Act
        let d = draft(&llm, &set(), &req).unwrap();

        // Assert
        assert_eq!(d.repairs, 0);
        assert_eq!(
            *llm.seen.borrow(),
            vec![1],
            "no repair request for a sample-only problem"
        );
        assert!(
            d.problems.iter().any(|p| p.contains("enough readable text")),
            "{:?}",
            d.problems
        );
    }

    #[test]
    fn the_last_compiled_recipe_is_kept_when_repairs_never_compile() {
        // Arrange
        let bad = GOOD.replace("h1.post-title", "h2.headline");
        let llm = FakeLlm::new(&[&bad, "not json", "not json", "not json"]);
        let page1 = long_page("Hello world");
        let also = [gate::Page {
            label: "page 2".into(),
            url: "https://blog.example/2026/second".into(),
            html: long_page("Second post"),
        }];
        let mut req = request();
        req.html = &page1;
        req.also = &also;
        req.repair_rounds = 1;

        // Act
        let d = draft(&llm, &set(), &req).unwrap();

        // Assert
        assert_eq!(d.repairs, 1);
        assert!(d.yaml.contains("h2.headline"), "{}", d.yaml);
        assert!(
            !d.problems.is_empty(),
            "the kept recipe still reports its gate problems"
        );
    }

    #[test]
    fn only_reviewed_recipes_are_offered_as_the_example() {
        // Arrange
        let mut s = set();
        s.add_yaml("id: other.example/post\nversion: 1\nschema: rex.schema.article.v1.Article\nextends: generic/article.v1\nmatch:\n  urls: { hosts: [other.example] }\nfields:\n  - path: title\n    candidates: [ { css: { selector: h1.unreviewed } } ]\n")
            .unwrap();
        let reviewed: BTreeSet<String> = ["generic/article.v1".to_string()].into();
        let mut req = request();
        req.reviewed = Some(&reviewed);

        // Act
        let pack = pack_for(&s, &req).unwrap();

        // Assert
        assert!(
            !pack.contains("h1.unreviewed"),
            "an unreviewed recipe was shown as the example"
        );
        assert!(pack.contains("generic/article.v1"), "the reviewed base is still named");
    }

    #[test]
    fn flatten_keeps_every_turn_in_order() {
        // Arrange
        let turns = [
            Turn {
                role: Role::User,
                text: "PACK".into(),
            },
            Turn {
                role: Role::Assistant,
                text: "{bad}".into(),
            },
            Turn {
                role: Role::User,
                text: "FIX IT".into(),
            },
        ];

        // Act
        let text = flatten("SYSTEM", &turns);

        // Assert
        let order: Vec<usize> = ["SYSTEM", "PACK", "Your previous answer was:\n{bad}", "FIX IT"]
            .iter()
            .map(|p| text.find(p).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{text}");
    }

    #[test]
    fn description_names_the_model_but_never_the_endpoint() {
        // Arrange
        let llm = HttpLlm::new(
            "http://private-host:6335/v1beta/models/flash:generateContent",
            Api::Gemini,
            "x",
            None,
            false,
        );

        // Act
        let d = llm.describe();

        // Assert
        assert_eq!(d, "Gemini (flash)");
    }

    #[test]
    fn transport_errors_never_name_the_endpoint_or_the_key() {
        // Arrange: nothing listens on port 9 of this host.
        let llm = HttpLlm::new(
            "http://127.0.0.1:9/v1beta/models/flash:generateContent",
            Api::Gemini,
            "x",
            Some("sk-secret".into()),
            false,
        );

        // Act
        let err = llm
            .complete(
                "system",
                &[Turn {
                    role: Role::User,
                    text: "hi".into(),
                }],
            )
            .unwrap_err();

        // Assert
        let text = format!("{err:#}");
        assert!(!text.contains("127.0.0.1") && !text.contains("sk-secret"), "{text}");
        assert!(text.contains("LLM endpoint"), "{text}");
    }

    #[test]
    fn evidence_pack_lists_schema_bindings_selectors_and_the_base() {
        // Arrange
        let registry = SchemaRegistry::embedded().unwrap();
        let spec = registry.record("rex.schema.article.v1.Article").unwrap();

        // Act
        let pack = evidence_pack(&PackInput {
            url: "https://www.blog.example/2026/hello",
            html: PAGE,
            schema: &spec,
            registry: &registry,
            id: "blog.example/post",
            base: Some("generic/article.v1"),
            example: None,
            urls: None,
        })
        .unwrap();

        // Assert
        assert!(pack.contains("extends: \"generic/article.v1\""));
        assert!(pack.contains("[\"blog.example\"]"));
        assert!(pack.contains("- title: string"), "{pack}");
        assert!(pack.contains("main#post"));
        assert!(pack.contains("h1.post-title \"Hello world\""));
        assert!(pack.contains("og:title = Hello"));
    }
}
