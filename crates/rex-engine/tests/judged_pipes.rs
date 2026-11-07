//! Pipes and recipe forms that keep the judge's errors out of records (decisions.md U44): `drop_codes`,
//! `unique { ignore_case }`, `not_future`, `filter { key }`, and omitting an inherited field.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rex_dom::{Document, Limits};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;

const BASE: &str = r#"
id: test/base
version: 1
schema: rex.schema.article.v1.Article
fields:
  - path: title
    candidates: [{ css: { selector: "h1" } }]
  - path: tags
    candidates: [{ meta: { key: "keywords" } }]
    pipe:
      - split: { pattern: "\\s*,\\s*" }
      - drop_codes: {}
      - unique: { ignore_case: true }
  - path: published_at
    candidates:
      - meta: { key: "article:published_time" }
      - css: { selector: "time[datetime]", take: TAKE_ATTRIBUTE, attribute: datetime }
    pipe: [{ not_future: {} }]
  - path: authors
    candidates:
      - json_ld: {}
      - meta: { key: "author" }
    pipe:
      - filter: { pattern: "^(https?:|www\\.)|^\\S+@\\S+\\.\\w+$", invert: true, key: name }
"#;

const SITE: &str = r#"
id: example.com/article
version: 1
schema: rex.schema.article.v1.Article
extends: test/base
match: { urls: { hosts: ["example.com"] } }
fields:
  - path: tags
    replace_inherited: true
"#;

fn record(recipes: &[&str], id: &str, html: &str) -> serde_json::Value {
    let mut set = RecipeSet::new(SchemaRegistry::embedded().unwrap());
    for r in recipes {
        set.add_yaml(r).unwrap();
    }
    let engine = Engine::new(set, Limits::default()).unwrap_or_else(|e| panic!("{e:?}"));
    let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-11-05T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let ctx = Ctx {
        now,
        fetch_time: Some(now),
        hints: BTreeMap::new(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "test".into(),
    };
    let doc = Document::parse(html, "https://example.com/a", &Limits::default()).unwrap();
    let result = engine.extract(id, &doc, &ctx).unwrap();
    engine.record_json(&result.record)
}

fn page(head: &str, body: &str) -> String {
    format!("<html><head>{head}</head><body><h1>Title</h1>{body}</body></html>")
}

#[test]
fn publisher_codes_and_case_duplicates_leave_the_tags() {
    // Arrange
    let html = page(
        r#"<meta name="keywords" content="LINK|i13-WP,Iran,l1:politics,iran,Elections,wc_401_600,G20">"#,
        "",
    );

    // Act
    let r = record(&[BASE], "test/base", &html);

    // Assert
    assert_eq!(r["tags"], serde_json::json!(["Iran", "Elections", "G20"]));
}

#[test]
fn a_future_date_falls_through_to_the_next_candidate() {
    // Arrange
    let html = page(
        r#"<meta property="article:published_time" content="2033-10-08T18:30:00Z">"#,
        r#"<time datetime="2026-11-04T09:00:00Z">Nov 4</time>"#,
    );

    // Act
    let r = record(&[BASE], "test/base", &html);

    // Assert
    assert_eq!(r["publishedAt"], "2026-11-04T09:00:00Z");
}

#[test]
fn an_email_as_the_json_ld_author_falls_through_to_the_meta_author() {
    // Arrange
    let html = page(
        r#"<script type="application/ld+json">{"@type":"Article","author":{"@type":"Person","name":"editor@example.com"}}</script>
           <meta name="author" content="Ada Lovelace">"#,
        "",
    );

    // Act
    let r = record(&[BASE], "test/base", &html);

    // Assert
    assert_eq!(r["authors"], serde_json::json!([{"name": "Ada Lovelace"}]));
}

#[test]
fn replace_inherited_without_candidates_leaves_the_field_unset() {
    // Arrange
    let html = page(r#"<meta name="keywords" content="Iran,Elections">"#, "");

    // Act
    let r = record(&[BASE, SITE], "example.com/article", &html);

    // Assert
    assert!(r.get("tags").is_none(), "{r}");
    assert_eq!(r["title"], "Title");
}
