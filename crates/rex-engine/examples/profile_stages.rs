//! Stage timing for a ~1 MB page (perf-free profiling). `cargo run --release -p rex-engine --example profile_stages`
use std::path::Path;
use std::time::Instant;

use chrono::DateTime;
use rex_dom::{Budget, Document, Limits};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;

fn main() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let src = std::fs::read_to_string(repo.join("fixtures/wsj.com/main/article/page.html")).unwrap();
    let para = "<p>Federal Reserve officials held interest rates steady on Wednesday, signaling patience as price growth slowed for a third straight month, analysts said.</p>\n<div class=\"ad\">Advertisement</div>\n";
    let html = src.replacen(
        "<p>Markets had largely",
        &format!("{}<p>Markets had largely", para.repeat(6500)),
        1,
    );
    let url = "https://www.wsj.com/articles/fed-holds-rates-steady-4f2a";
    let mut set = RecipeSet::new(SchemaRegistry::embedded().unwrap());
    for f in ["recipes/generic/article.v1.yaml", "recipes/sites/wsj.com/main.yaml"] {
        set.add_yaml(&std::fs::read_to_string(repo.join(f)).unwrap()).unwrap();
    }
    let engine = Engine::new(set, Limits::default()).unwrap();
    let ctx = Ctx {
        now: DateTime::from_timestamp(1_791_727_200, 0).unwrap(),
        fetch_time: None,
        hints: Default::default(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "p".into(),
    };
    let t = |label: &str, f: &mut dyn FnMut()| {
        let s = Instant::now();
        f();
        println!("{label:40} {:>8.1} ms", s.elapsed().as_secs_f64() * 1000.0);
    };
    let doc = Document::parse(&html, url, &Limits::default()).unwrap();
    t("parse", &mut || {
        Document::parse(&html, url, &Limits::default()).unwrap();
    });
    t("readability", &mut || {
        rex_readability::extract(&doc, &Budget::new(u64::MAX)).unwrap();
    });
    t("block_text(article)", &mut || {
        let sel = rex_dom::Sel::parse("article", 512).unwrap();
        let el = doc.select(doc.root(), &sel, &Budget::new(u64::MAX)).unwrap()[0];
        rex_dom::text::block_text(el, &Budget::new(u64::MAX)).unwrap();
    });
    let inner = {
        let sel = rex_dom::Sel::parse("section[subscriptions-section=content]", 512).unwrap();
        doc.select(doc.root(), &sel, &Budget::new(u64::MAX)).unwrap()[0].inner_html()
    };
    t("ammonia clean (section inner html)", &mut || {
        ammonia::clean(&inner);
    });
    t("parse_fragment (section inner html)", &mut || {
        scraper::Html::parse_fragment(&inner);
    });
    t("extract generic/article.v1", &mut || {
        engine.extract("generic/article.v1", &doc, &ctx).unwrap();
    });
    t("extract wsj.com/main", &mut || {
        engine.extract("wsj.com/main", &doc, &ctx).unwrap();
    });
    t("plan (match)", &mut || {
        engine.plan(&doc);
    });
}
