//! Engine-only benchmark (engine spec §11): parse + match + extract a ~1 MB article page with the
//! wsj.com/main recipe (which forces readability for the body agreement metric).

use std::path::Path;

use chrono::DateTime;
use criterion::{criterion_group, criterion_main, Criterion};
use rex_dom::{Document, Limits};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;

const PARAGRAPH: &str = "<p>Federal Reserve officials held interest rates steady on Wednesday, signaling patience as price growth slowed for a third straight month, analysts said.</p>\n<div class=\"ad\">Advertisement</div>\n";

pub fn big_page(repo: &Path) -> String {
    let src = std::fs::read_to_string(repo.join("fixtures/wsj.com/main/article/page.html")).expect("fixture");
    let filler = PARAGRAPH.repeat(6500);
    src.replacen("<p>Markets had largely", &format!("{filler}<p>Markets had largely"), 1)
}

fn engine(repo: &Path) -> Engine {
    let mut set = RecipeSet::new(SchemaRegistry::embedded().expect("schemas"));
    for f in ["recipes/generic/article.v1.yaml", "recipes/sites/wsj.com/main.yaml"] {
        set.add_yaml(&std::fs::read_to_string(repo.join(f)).expect("recipe"))
            .expect("parse");
    }
    Engine::new(set, Limits::default()).expect("compile")
}

fn bench(c: &mut Criterion) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let html = big_page(&repo);
    let engine = engine(&repo);
    let ctx = Ctx {
        now: DateTime::from_timestamp(1_791_727_200, 0).expect("time"),
        fetch_time: None,
        hints: Default::default(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "bench".into(),
    };
    let url = "https://www.wsj.com/articles/fed-holds-rates-steady-4f2a";
    eprintln!("page size: {} bytes", html.len());
    let mut group = c.benchmark_group("extract");
    group.sample_size(10);
    group.bench_function("parse_1mb", |b| {
        b.iter(|| Document::parse(&html, url, engine.limits()).expect("parse"))
    });
    group.bench_function("parse_match_extract_1mb", |b| {
        b.iter(|| {
            let doc = Document::parse(&html, url, engine.limits()).expect("parse");
            let (_, r) = engine.extract_auto(&doc, &ctx);
            r.expect("extracted")
        })
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
