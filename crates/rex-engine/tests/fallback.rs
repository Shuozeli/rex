//! The schema.org fallback picks the page's main type by `fallback_rank` (decisions.md U36), using the
//! repository's generic recipes.

use std::path::Path;

use rex_dom::Limits;
use rex_engine::{Engine, RecipeSet};
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

fn types(ts: &[&str]) -> Vec<String> {
    ts.iter().map(|t| t.to_string()).collect()
}

#[test]
fn specific_entity_outranks_an_earlier_hero_video() {
    // Arrange
    let engine = generic_engine();

    // Act
    let chosen = engine.fallback_for(&types(&["VideoObject", "Recipe", "BreadcrumbList"]));

    // Assert
    assert_eq!(chosen.as_deref(), Some("generic/recipe.v1"));
}

#[test]
fn faq_block_ranks_below_the_page_entity() {
    // Arrange
    let engine = generic_engine();

    // Act
    let chosen = engine.fallback_for(&types(&["WebPage", "FAQPage", "Event"]));

    // Assert
    assert_eq!(chosen.as_deref(), Some("generic/event.v1"));
}

#[test]
fn equal_ranks_keep_document_order() {
    // Arrange
    let engine = generic_engine();

    // Act
    let chosen = engine.fallback_for(&types(&["NewsArticle", "VideoObject"]));

    // Assert
    assert_eq!(chosen.as_deref(), Some("generic/article.v1"));
}
