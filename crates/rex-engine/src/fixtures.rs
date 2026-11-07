//! Fixture runner shared by `rex test` and the conformance tests.
//!
//! A fixture directory holds `page.html`, `meta.json` and `expected.json` (the Extraction protojson).
//! An optional `recipe.yaml` adds a case-local recipe (conformance cases); otherwise the case runs
//! against the shared recipe set, either the recipe named in `meta.json` or the matched one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rex_dom::{Document, Limits};
use serde::{Deserialize, Serialize};

use crate::{Ctx, Engine, RecipeSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureMeta {
    pub url: String,
    pub fetch_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hints: BTreeMap<String, String>,
    /// Hand-written page (no publisher content): exempt from the redaction lint.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub synthetic: bool,
    /// Judge findings accepted for this page, each with the reason (decisions.md U44).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub judge_allow: Vec<JudgeAllow>,
}

/// One accepted judge finding: its rule, optionally a path prefix, and why it is right for this page.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct JudgeAllow {
    pub rule: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub reason: String,
}

impl JudgeAllow {
    pub fn covers(&self, f: &crate::judge::Finding) -> bool {
        self.rule == f.rule && self.path.as_deref().is_none_or(|p| f.path.starts_with(p))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Updated,
    Fail { diff: String },
    Error(String),
}

/// A fixture's page: `page.html`, or `page.html.gz` (fixtures stored in the rex-fixtures repo, U31).
pub fn read_page(dir: &Path) -> Result<String, String> {
    let plain = dir.join("page.html");
    if plain.is_file() {
        return std::fs::read_to_string(&plain).map_err(|e| format!("page.html: {e}"));
    }
    let gz = dir.join("page.html.gz");
    #[cfg(feature = "gz")]
    {
        use std::io::Read;
        let file = std::fs::File::open(&gz).map_err(|e| format!("page.html(.gz): {e}"))?;
        let mut text = String::new();
        flate2::read::GzDecoder::new(file)
            .read_to_string(&mut text)
            .map_err(|e| format!("page.html.gz: {e}"))?;
        Ok(text)
    }
    #[cfg(not(feature = "gz"))]
    Err(format!("{} needs the `gz` feature", gz.display()))
}

/// Every directory under `root` that contains a `meta.json`, sorted.
pub fn discover(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    if dir.join("meta.json").is_file() {
        out.push(dir.to_path_buf());
    }
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        }
    }
}

/// Run one fixture. `shared` is the engine over the shared recipe set; `base` is that set (cloned
/// when a case brings its own recipe).
pub fn run_case(shared: &Engine, base: &RecipeSet, limits: &Limits, dir: &Path, update: bool) -> Outcome {
    match produce_judged(shared, base, limits, dir) {
        Err(e) => Outcome::Error(e),
        Ok((actual, findings)) => {
            // The output must also be good, not only unchanged: judge errors fail the case (the expected file
            // is still written under --update, so the diff can be reviewed).
            let errors: Vec<String> = findings
                .iter()
                .filter(|f| f.level == crate::judge::Level::Error)
                .map(|f| f.to_string())
                .collect();
            if !errors.is_empty() {
                if update {
                    let _ = std::fs::write(dir.join("expected.json"), &actual);
                }
                return Outcome::Fail {
                    diff: format!("judge:\n  {}", errors.join("\n  ")),
                };
            }
            let expected_path = dir.join("expected.json");
            let expected = std::fs::read_to_string(&expected_path).ok();
            if expected.as_deref() == Some(actual.as_str()) {
                return Outcome::Pass;
            }
            if update {
                return match std::fs::write(&expected_path, &actual) {
                    Ok(()) => Outcome::Updated,
                    Err(e) => Outcome::Error(format!("cannot write {}: {e}", expected_path.display())),
                };
            }
            Outcome::Fail {
                diff: diff(expected.as_deref().unwrap_or(""), &actual),
            }
        }
    }
}

/// The Extraction protojson a fixture produces (pretty, trailing newline).
pub fn produce(shared: &Engine, base: &RecipeSet, limits: &Limits, dir: &Path) -> Result<String, String> {
    produce_judged(shared, base, limits, dir).map(|(json, _)| json)
}

/// `produce`, plus the judge's findings on the record (those the fixture's `judge_allow` accepts are dropped).
pub fn produce_judged(
    shared: &Engine,
    base: &RecipeSet,
    limits: &Limits,
    dir: &Path,
) -> Result<(String, Vec<crate::judge::Finding>), String> {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"));
    let meta: FixtureMeta = serde_json::from_str(&read("meta.json")?).map_err(|e| format!("meta.json: {e}"))?;
    let html = read_page(dir)?;
    let now: DateTime<Utc> = DateTime::parse_from_rfc3339(&meta.fetch_time)
        .map_err(|e| format!("meta.json fetch_time: {e}"))?
        .with_timezone(&Utc);
    let ctx = Ctx {
        now,
        fetch_time: Some(now),
        hints: meta.hints.clone(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "test".into(),
    };
    let doc = Document::parse(&html, &meta.url, limits).map_err(|e| e.to_string())?;

    let local_engine;
    let (engine, explicit) = if dir.join("recipe.yaml").is_file() {
        let mut set = base.clone();
        let id = set.add_yaml(&read("recipe.yaml")?).map_err(|e| e.to_string())?;
        local_engine = Engine::new(set, limits.clone())
            .map_err(|errs| errs.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))?;
        (&local_engine, Some(meta.recipe.clone().unwrap_or(id)))
    } else {
        (shared, meta.recipe.clone())
    };
    let result = match explicit {
        Some(id) => engine
            .extract(&id, &doc, &ctx)
            .ok_or_else(|| format!("unknown recipe `{id}`"))?,
        None => engine.extract_auto(&doc, &ctx).1.ok_or("no recipe matched the page")?,
    };
    let json = engine.to_json(&result.extraction);
    let record = engine.record_json(&result.record);
    let page = crate::judge::PageFacts::of(&doc, now);
    let findings = crate::judge::judge(engine.recipes().registry(), &result.extraction.schema, &record, &page)
        .into_iter()
        .filter(|f| !meta.judge_allow.iter().any(|a| a.covers(f)))
        .collect();
    Ok((
        serde_json::to_string_pretty(&json).map_err(|e| e.to_string())? + "\n",
        findings,
    ))
}

fn diff(expected: &str, actual: &str) -> String {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let first = e
        .iter()
        .zip(a.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(e.len().min(a.len()));
    let from = first.saturating_sub(2);
    let mut out = format!("first difference at line {}\n", first + 1);
    for i in from..(first + 4).min(e.len().max(a.len())) {
        if let Some(l) = e.get(i) {
            out.push_str(&format!("- {l}\n"));
        }
        if let Some(l) = a.get(i) {
            out.push_str(&format!("+ {l}\n"));
        }
    }
    out
}
