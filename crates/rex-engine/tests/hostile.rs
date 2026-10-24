//! Hostile pages against the repository's generic recipes: each must finish quickly, with bounded memory, and
//! without panicking (the wasm build aborts on panic). Code review 2026-10-16.

use std::path::Path;
use std::time::{Duration, Instant};

use rex_dom::{Document, Limits};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;

fn generic_engine() -> Engine {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/generic");
    let mut set = RecipeSet::new(SchemaRegistry::embedded().expect("schemas"));
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("generic dir")
        .map(|e| e.expect("entry").path())
        .collect();
    files.sort();
    for f in files {
        set.add_yaml(&std::fs::read_to_string(&f).expect("read recipe"))
            .expect("recipe");
    }
    Engine::new(set, Limits::default()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn ctx() -> Ctx {
    Ctx {
        now: chrono::DateTime::parse_from_rfc3339("2026-10-16T00:00:00Z")
            .unwrap()
            .into(),
        fetch_time: None,
        hints: Default::default(),
        source_id: "test".into(),
        source_ref: "test".into(),
        host: "test".into(),
    }
}

/// Runs plan + extract on `html`; returns the elapsed time.
fn run(html: &str) -> Duration {
    let engine = generic_engine();
    let started = Instant::now();
    let doc = Document::parse(html, "https://e.com/a", engine.limits()).expect("parses");
    let _ = engine.extract_auto(&doc, &ctx());
    started.elapsed()
}

#[test]
fn json_ld_reference_fan_out_is_bounded() {
    // Arrange: a 500 KB Person referenced 3000 times by an article's author list
    let big = "x".repeat(500_000);
    let refs = vec![r#"{"@id":"p"}"#; 3000].join(",");
    let html = format!(
        r#"<html><head><script type="application/ld+json">{{"@id":"p","@type":"Person","name":"{big}"}}</script>
        <script type="application/ld+json">{{"@type":"NewsArticle","headline":"h","author":[{refs}]}}</script></head><body><p>x</p></body></html>"#
    );

    // Act
    let took = run(&html);

    // Assert
    assert!(took < Duration::from_secs(10), "took {took:?}");
}

#[test]
fn thousands_of_declared_types_do_not_slow_matching() {
    // Arrange: one JSON-LD array of 40k nodes with distinct @type
    let nodes: Vec<String> = (0..40_000).map(|i| format!(r#"{{"@type":"T{i}"}}"#)).collect();
    let html = format!(
        r#"<script type="application/ld+json">[{}]</script><p>x</p>"#,
        nodes.join(",")
    );

    // Act
    let took = run(&html);

    // Assert
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    // Arrange: 5k nested divs (far past the 256-level prune; html5ever itself is slow on deeper input in
    // debug builds) and no title, so generic/article falls back to readability
    let html = format!(
        "<html><body>{}text{}</body></html>",
        "<div>".repeat(5_000),
        "</div>".repeat(5_000)
    );

    // Act
    let took = run(&html);

    // Assert: reaching this line at all means no stack overflow
    assert!(took < Duration::from_secs(30), "took {took:?}");
}

#[test]
fn nested_matches_do_not_build_quadratic_html() {
    // Arrange: 200 nested article elements each with text, which body selectors serialize as HTML
    let inner: String = (0..200)
        .map(|i| format!("<article><p>paragraph {i} {}</p>", "y".repeat(2000)))
        .collect();
    let html = format!(
        r#"<html><head><meta property="og:type" content="article"><title>t</title></head><body>{inner}{}</body></html>"#,
        "</article>".repeat(200)
    );

    // Act
    let took = run(&html);

    // Assert
    assert!(took < Duration::from_secs(10), "took {took:?}");
}
