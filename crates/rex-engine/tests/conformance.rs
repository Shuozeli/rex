//! Runs every fixture (`fixtures/`) and conformance case (`conformance/`) against `recipes/`, natively.
//! The same cases run under wasm in M2 (engine spec §14).

use std::path::{Path, PathBuf};

use rex_dom::Limits;
use rex_engine::fixtures::{discover, run_case, Outcome};
use rex_engine::{Engine, RecipeSet};
use rex_schema::SchemaRegistry;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn recipe_set() -> RecipeSet {
    let mut set = RecipeSet::new(SchemaRegistry::embedded().expect("schemas"));
    let mut files = Vec::new();
    collect(&repo().join("recipes"), &mut files);
    files.sort();
    for f in files {
        set.add_yaml(&std::fs::read_to_string(&f).expect("read recipe"))
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    }
    set
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir").flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "yaml") {
            out.push(p);
        }
    }
}

#[test]
fn every_fixture_and_conformance_case_matches_expected() {
    // Arrange
    let set = recipe_set();
    let limits = Limits::default();
    let engine = Engine::new(set.clone(), limits.clone()).expect("recipes compile");
    let cases: Vec<PathBuf> = ["fixtures", "conformance"]
        .iter()
        .flat_map(|r| discover(&repo().join(r)))
        .collect();
    assert!(cases.len() >= 14, "expected the committed cases, found {}", cases.len());

    // Act
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|dir| match run_case(&engine, &set, &limits, dir, false) {
            Outcome::Pass => None,
            other => Some(format!("{}: {other:?}", dir.display())),
        })
        .collect();

    // Assert
    assert!(failures.is_empty(), "failing cases:\n{}", failures.join("\n"));
}
