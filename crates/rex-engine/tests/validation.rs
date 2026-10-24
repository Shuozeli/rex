//! Compile-time recipe validation added after the 2026-10-16 code review.

use rex_dom::Limits;
use rex_engine::RecipeSet;
use rex_schema::SchemaRegistry;

/// Compiles `yaml` (plus an optional base) and returns the error messages.
fn errors(yaml: &str, base: Option<&str>) -> Vec<String> {
    let mut set = RecipeSet::new(SchemaRegistry::embedded().expect("schemas"));
    if let Some(b) = base {
        set.add_yaml(b).expect("base parses");
    }
    let id = set.add_yaml(yaml).expect("recipe parses");
    match set.compile(&id, &Limits::default()) {
        Ok(_) => Vec::new(),
        Err(es) => es.into_iter().map(|e| e.to_string()).collect(),
    }
}

const HEAD: &str = "version: 1\nschema: rex.schema.article.v1.Article\nmatch:\n  urls:\n    hosts: [\"e.com\"]\n";

#[test]
fn relative_dates_without_the_feature_is_an_error() {
    // Arrange
    let yaml = format!(
        "id: e.com/a\n{HEAD}fields:\n  - path: title\n    candidates: [{{ css: {{ selector: h1 }} }}]\n  - path: published_at\n    candidates: [{{ css: {{ selector: time }} }}]\n    pipe: [{{ datetime: {{ relative: true }} }}]\n"
    );

    // Act
    let errs = errors(&yaml, None);

    // Assert
    assert!(errs.iter().any(|e| e.contains("relative_dates")), "{errs:?}");
}

#[test]
fn a_default_that_cannot_fit_its_field_is_an_error() {
    // Arrange: a timestamp field defaulting to text
    let yaml = format!(
        "id: e.com/a\n{HEAD}fields:\n  - path: title\n    candidates: [{{ css: {{ selector: h1 }} }}]\n  - path: published_at\n    candidates: [{{ css: {{ selector: time }} }}]\n    pipe: [{{ default: {{ value: \"not a date\" }} }}]\n"
    );

    // Act
    let errs = errors(&yaml, None);

    // Assert
    assert!(errs.iter().any(|e| e.contains("does not fit")), "{errs:?}");
}

#[test]
fn a_bad_microdata_path_is_an_error() {
    // Arrange
    let yaml = format!(
        "id: e.com/a\n{HEAD}fields:\n  - path: title\n    candidates: [{{ microdata: {{ path: \"name[\" }} }}]\n"
    );

    // Act
    let errs = errors(&yaml, None);

    // Assert
    assert!(!errs.is_empty(), "an unclosed `[` must not compile");
}
