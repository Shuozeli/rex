//! JSON-LD index (engine spec §4): every `application/ld+json` block, arrays and `@graph` flattened,
//! `@id` references resolvable one level. Paths are dotted with `[n]` / `[*]`; arrays are implicitly
//! flattened at every step (JSON-LD freely mixes single values and arrays).

use scraper::{Html, Selector};
use serde_json::Value;

/// Cap on a single JSON-LD block (bytes).
const MAX_BLOCK_BYTES: usize = 1024 * 1024;

#[derive(Debug, Default)]
pub struct JsonLdIndex {
    pub nodes: Vec<Value>,
    pub parse_errors: usize,
}

impl JsonLdIndex {
    pub fn build(html: &Html) -> JsonLdIndex {
        let sel = Selector::parse("script[type]").expect("static selector");
        let mut idx = JsonLdIndex::default();
        for s in html.select(&sel) {
            let ty = s.value().attr("type").unwrap_or_default();
            if !ty.trim().eq_ignore_ascii_case("application/ld+json") {
                continue;
            }
            let raw: String = s.text().collect();
            if raw.len() > MAX_BLOCK_BYTES {
                idx.parse_errors += 1;
                continue;
            }
            match parse_lenient(&raw) {
                Some(v) => flatten_into(v, &mut idx.nodes),
                None => idx.parse_errors += 1,
            }
        }
        idx
    }

    /// Nodes whose `@type` intersects `types` (case-sensitive schema.org names; a full IRI matches
    /// on its last segment), in document order.
    pub fn nodes_of_types<'a>(&'a self, types: &'a [String]) -> impl Iterator<Item = &'a Value> + 'a {
        self.nodes
            .iter()
            .filter(move |n| node_types(n).iter().any(|t| types.iter().any(|w| w == t)))
    }

    /// All `@type`s present, in document order (for the schema.org fallback).
    pub fn all_types(&self) -> Vec<String> {
        self.nodes.iter().flat_map(node_types).collect()
    }

    /// Replace a bare `{"@id": …}` reference with the full node carrying that id, if any.
    pub fn resolve<'a>(&'a self, v: &'a Value) -> &'a Value {
        let Some(obj) = v.as_object() else { return v };
        let Some(id) = obj.get("@id").and_then(Value::as_str) else {
            return v;
        };
        if obj.keys().any(|k| k != "@id" && k != "@type") {
            return v;
        }
        self.nodes
            .iter()
            .find(|n| n.get("@id").and_then(Value::as_str) == Some(id) && n.as_object().is_some_and(|o| o.len() > 2))
            .unwrap_or(v)
    }

    /// `eval_path` for hostile pages: spends fuel for every value visited and for the size of every value it
    /// copies out (a bare `{"@id": ..}` reference expands to the full node, so 3000 references to one 1 MB
    /// node would otherwise copy 3 GB for free), and returns at most `max_results` values.
    pub fn eval_path_bounded(
        &self,
        node: &Value,
        path: &str,
        budget: &crate::Budget,
        max_results: usize,
    ) -> Result<Vec<Value>, crate::BudgetExceeded> {
        let Ok(steps) = parse_path(path) else {
            return Ok(Vec::new());
        };
        let mut current: Vec<&Value> = vec![node];
        for step in &steps {
            let mut next = Vec::new();
            for v in current {
                budget.spend(1)?;
                let v = self.resolve(v);
                for item in spread(v) {
                    let item = self.resolve(item);
                    if let Some(child) = item.get(&step.key) {
                        match step.index {
                            Index::All => next.extend(spread(child)),
                            Index::At(i) => {
                                if let Some(x) = spread(child).get(i) {
                                    next.push(*x);
                                }
                            }
                        }
                    }
                }
            }
            current = next;
        }
        let mut out = Vec::new();
        for v in current.into_iter().flat_map(spread) {
            if out.len() >= max_results {
                break;
            }
            let v = self.resolve(v);
            budget.spend(1 + value_bytes(v) / 16)?;
            out.push(v.clone());
        }
        Ok(out)
    }

    /// Evaluate a dotted path against `node`; returns the leaf values in order.
    pub fn eval_path(&self, node: &Value, path: &str) -> Result<Vec<Value>, String> {
        let steps = parse_path(path)?;
        let mut current: Vec<&Value> = vec![node];
        for step in &steps {
            let mut next = Vec::new();
            for v in current {
                let v = self.resolve(v);
                for item in spread(v) {
                    let item = self.resolve(item);
                    if let Some(child) = item.get(&step.key) {
                        match step.index {
                            Index::All => next.extend(spread(child)),
                            Index::At(i) => {
                                let items = spread(child);
                                if let Some(x) = items.get(i) {
                                    next.push(*x);
                                }
                            }
                        }
                    }
                }
            }
            current = next;
        }
        Ok(current
            .into_iter()
            .flat_map(spread)
            .map(|v| self.resolve(v).clone())
            .collect())
    }
}

#[derive(Debug, PartialEq)]
enum Index {
    All,
    At(usize),
}

#[derive(Debug)]
struct Step {
    key: String,
    index: Index,
}

fn parse_path(path: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    for seg in path.split('.') {
        let seg = seg.trim();
        if seg.is_empty() {
            return Err(format!("empty segment in JSON path `{path}`"));
        }
        if let Some(open) = seg.find('[') {
            let key = &seg[..open];
            let rest = seg[open + 1..]
                .strip_suffix(']')
                .ok_or_else(|| format!("unclosed `[` in `{path}`"))?;
            let index = if rest == "*" {
                Index::All
            } else {
                Index::At(
                    rest.parse::<usize>()
                        .map_err(|_| format!("bad index `{rest}` in `{path}`"))?,
                )
            };
            steps.push(Step {
                key: key.to_string(),
                index,
            });
        } else {
            steps.push(Step {
                key: seg.to_string(),
                index: Index::All,
            });
        }
    }
    Ok(steps)
}

/// Validate a JSON path at recipe compile time.
pub fn validate_path(path: &str) -> Result<(), String> {
    parse_path(path).map(|_| ())
}

/// Approximate size of a JSON value in bytes (strings and keys plus a few bytes per node), for fuel.
pub fn value_bytes(v: &Value) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        total += 8;
        match v {
            Value::String(s) => total += s.len() as u64,
            Value::Array(a) => stack.extend(a.iter()),
            Value::Object(o) => {
                for (k, x) in o {
                    total += k.len() as u64;
                    stack.push(x);
                }
            }
            _ => {}
        }
    }
    total
}

fn spread(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

pub fn node_types(n: &Value) -> Vec<String> {
    let short = |s: &str| s.rsplit(['/', '#', ':']).next().unwrap_or(s).to_string();
    match n.get("@type") {
        Some(Value::String(s)) => vec![short(s)],
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(short).collect(),
        _ => Vec::new(),
    }
}

fn flatten_into(v: Value, out: &mut Vec<Value>) {
    match v {
        Value::Array(items) => {
            for i in items {
                flatten_into(i, out);
            }
        }
        Value::Object(mut obj) => {
            if let Some(graph) = obj.remove("@graph") {
                flatten_into(graph, out);
            }
            if obj.contains_key("@type") {
                out.push(Value::Object(obj));
            }
        }
        _ => {}
    }
}

fn parse_lenient(raw: &str) -> Option<Value> {
    let trimmed = raw
        .trim()
        .trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim()
        .trim_start_matches("//<![CDATA[")
        .trim_end_matches("//]]>")
        .trim();
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Some(v);
    }
    // Raw control characters inside strings are the most common breakage.
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.replace(['\n', '\t'], " ");
    serde_json::from_str(&cleaned).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_reference_fan_out_is_charged_and_capped() {
        // Arrange: one 200 KB Person referenced 3000 times
        let big = "x".repeat(200_000);
        let html = format!(
            r#"<script type="application/ld+json">{{"@id":"p","@type":"Person","name":"{big}"}}</script>
               <script type="application/ld+json">{{"@type":"NewsArticle","author":[{refs}]}}</script>"#,
            refs = vec![r#"{"@id":"p"}"#; 3000].join(",")
        );
        let idx = JsonLdIndex::build(&Html::parse_document(&html));
        let types = vec!["NewsArticle".to_string()];
        let node = idx.nodes_of_types(&types).next().expect("article node");
        let budget = crate::Budget::new(5_000_000);

        // Act
        let res = idx.eval_path_bounded(node, "author.name", &budget, 1000);

        // Assert: the copies are paid for, so the budget stops the fan-out long before 600 MB is built
        assert!(res.is_err(), "fan-out must exhaust the budget");
        assert!(budget.used() > 5_000_000);
    }

    fn index(html: &str) -> JsonLdIndex {
        JsonLdIndex::build(&Html::parse_document(html))
    }

    #[test]
    fn graph_is_flattened_and_ids_resolved() {
        // Arrange
        let idx = index(
            r##"<script type="application/ld+json">{"@context":"https://schema.org","@graph":[
                {"@type":"NewsArticle","headline":"H","author":{"@id":"#p1"}},
                {"@type":"Person","@id":"#p1","name":"Jane Doe","url":"/jane"}]}</script>"##,
        );
        let types = vec!["NewsArticle".to_string()];

        // Act
        let node = idx.nodes_of_types(&types).next().expect("article node");
        let names = idx.eval_path(node, "author.name").unwrap();

        // Assert
        assert_eq!(idx.nodes.len(), 2);
        assert_eq!(names, vec![Value::String("Jane Doe".into())]);
    }

    #[test]
    fn arrays_flatten_and_indexes_select() {
        // Arrange
        let idx = index(
            r#"<script type="application/ld+json">{"@type":["BlogPosting"],"image":[{"url":"a.png"},{"url":"b.png"}],
                "author":[{"name":"A"},{"name":"B"}]}</script>"#,
        );
        let node = &idx.nodes[0];

        // Act
        let first_image = idx.eval_path(node, "image[0].url").unwrap();
        let authors = idx.eval_path(node, "author[*].name").unwrap();

        // Assert
        assert_eq!(first_image, vec![Value::String("a.png".into())]);
        assert_eq!(authors.len(), 2);
    }

    #[test]
    fn broken_block_counts_as_error() {
        // Arrange / Act
        let idx = index(r#"<script type="application/ld+json">{"@type": "Article", </script>"#);

        // Assert
        assert_eq!(idx.parse_errors, 1);
        assert!(idx.nodes.is_empty());
    }
}
