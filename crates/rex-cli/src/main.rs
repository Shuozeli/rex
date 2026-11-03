//! `rex`: check, extract, explain, match and test recipes (design.md §7.1).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use rex_dom::{Document, Limits};
use rex_engine::fixtures::{self, Outcome};
use rex_engine::{Ctx, Engine, RecipeSet};
use rex_schema::SchemaRegistry;
use ureq::ResponseExt;

mod draft;
mod gate;
mod publish;

#[derive(Parser)]
#[command(
    name = "rex",
    version,
    about = "Turn web pages into typed records with declarative recipes"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate and compile recipes (contract, paths, selectors, regexes, extends, policy).
    Check {
        /// Recipe files or directories (searched recursively for .yaml/.yml).
        #[arg(default_value = "recipes")]
        paths: Vec<PathBuf>,
        /// Also require >= 2 fixture cases per site recipe, counted across these roots (repeatable: the main
        /// repo's fixtures/ and the rex-fixtures repo).
        #[arg(long)]
        fixtures: Vec<PathBuf>,
        /// Version gate: a recipe whose canonical content changed since this git ref must bump
        /// `version` by exactly 1; an unchanged recipe must keep its version (FR-RC-4).
        #[arg(long)]
        git_base: Option<String>,
    },
    /// Capture, redact and lint fixtures (decisions.md U19).
    #[command(subcommand)]
    Fixture(FixtureCmd),
    /// Extract a page and print the Extraction (protojson).
    Extract(PageArgs),
    /// Extract a page and print the per-field candidate trace.
    Explain(PageArgs),
    /// Show how a URL (and optionally its HTML) matches recipes.
    Match {
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        #[arg(long)]
        url: String,
        #[arg(long)]
        html: Option<PathBuf>,
    },
    /// Run only the readability source on a page and print title/byline/excerpt/contentText (JSON).
    Readability {
        #[arg(long)]
        html: PathBuf,
        #[arg(long)]
        url: String,
    },
    /// Fetch the live sample pages in liveness.yaml and verify recipe + status (decisions.md U20).
    Liveness {
        #[arg(long, default_value = "liveness.yaml")]
        file: PathBuf,
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        /// Write a JSON report (for the liveness workflow) to this path.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Pages fetched in parallel (extraction stays on one thread).
        #[arg(long, default_value_t = 8)]
        jobs: usize,
        /// Skip the checks marked `datacenter_blocked` (CI runners: the site refuses their IPs), reporting
        /// them as skipped with the reason.
        #[arg(long)]
        skip_datacenter_blocked: bool,
    },
    /// Print a recipe in canonical form (protojson with --json).
    Fmt {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Build, sign and verify recipe bundles (rex.bundle.v1).
    #[command(subcommand)]
    Bundle(BundleCmd),
    /// Draft a recipe for one page with a BYO LLM (design.md §7.2) into recipes/drafts/. The draft is
    /// unreviewed: it must pass `rex test` and review like any recipe.
    Draft {
        /// A fixture directory (page.html + meta.json) or an HTML file (then --url is required).
        #[arg(long)]
        from: PathBuf,
        /// Page URL (default: meta.json `url`).
        #[arg(long)]
        url: Option<String>,
        /// Target schema message, e.g. rex.schema.article.v1.Article.
        #[arg(long, default_value = "rex.schema.article.v1.Article")]
        schema: String,
        /// Recipe id (default: <host>/<schema short name>).
        #[arg(long)]
        id: Option<String>,
        /// Full endpoint URL: .../chat/completions (OpenAI-compatible) or ...:generateContent (Gemini).
        #[arg(long, env = "REX_LLM_ENDPOINT", required_unless_present = "pack_only")]
        llm: Option<String>,
        /// Model name for OpenAI-compatible endpoints (Gemini takes it from the URL).
        #[arg(long, env = "REX_LLM_MODEL", default_value = "default")]
        model: String,
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        #[arg(long, default_value = "recipes/drafts")]
        out: PathBuf,
        /// More fixture directories of the same kind of page; the quality gate runs on them too.
        #[arg(long)]
        also: Vec<PathBuf>,
        /// Fixture directories of pages the recipe must not match (e.g. the homepage).
        #[arg(long)]
        negative: Vec<PathBuf>,
        /// Send quality-gate failures back to the model this many times (batch harvesting uses 2; U31).
        #[arg(long, default_value_t = 0)]
        repair_rounds: usize,
        /// Fix the URL rules instead of letting the model write them: the hosts (repeatable) ...
        #[arg(long)]
        url_host: Vec<String>,
        /// ... and include regexes (repeatable), e.g. computed from real article URLs.
        #[arg(long)]
        url_include: Vec<String>,
        /// Write the draft even if it fails the quality gate (the problems are printed).
        #[arg(long)]
        keep_failing: bool,
        /// Send everything (system prompt and conversation) as one user message, for endpoints or proxies without
        /// system roles or multi-turn support.
        #[arg(long, env = "REX_LLM_INLINE_SYSTEM")]
        inline_system: bool,
        /// Print the evidence pack and exit without calling the model.
        #[arg(long)]
        pack_only: bool,
        /// Replace an existing draft file.
        #[arg(long)]
        force: bool,
    },
    /// Quality gate (U31) for one recipe on captured pages; prints a JSON report, exit 1 on failure.
    Gate {
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        #[arg(long)]
        id: String,
        /// Fixture directories of pages the recipe is for.
        #[arg(long, required = true)]
        page: Vec<PathBuf>,
        /// Fixture directories of pages it must not match.
        #[arg(long)]
        negative: Vec<PathBuf>,
    },
    /// Run fixture cases and compare against expected.json.
    Test {
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        /// Fixture roots (default: fixtures and conformance, when present).
        #[arg(long = "fixtures")]
        fixtures: Vec<PathBuf>,
        /// Rewrite expected.json for failing or missing cases.
        #[arg(long)]
        update: bool,
        /// Only run cases whose path contains one of these substrings.
        filter: Vec<String>,
    },
}

#[derive(Subcommand)]
enum FixtureCmd {
    /// Fetch a page over HTTP into fixtures-private/<id>/<case>/ (never committed).
    Capture {
        url: String,
        /// Recipe id the fixture belongs to (directory under the fixture root).
        #[arg(long)]
        id: String,
        #[arg(long)]
        case: String,
        #[arg(long, default_value = "fixtures-private")]
        root: PathBuf,
    },
    /// Redact a captured case into a committable fixture directory (page.html + meta.json).
    Redact {
        from: PathBuf,
        to: PathBuf,
        /// Write page.html.gz (fixtures in the rex-fixtures repo).
        #[arg(long)]
        gzip: bool,
    },
    /// Add a fixture saved by the extension's developer mode (*.rexfixture.json): re-lints the redaction and
    /// writes ROOT/<recipe>/<case>/{page.html, meta.json}.
    Import {
        file: PathBuf,
        #[arg(long, default_value = "fixtures")]
        root: PathBuf,
        /// Recipe id (default: the one recorded in the file).
        #[arg(long)]
        recipe: Option<String>,
        /// Case name (default: derived from the page URL).
        #[arg(long)]
        case: Option<String>,
    },
    /// Fail if any non-synthetic fixture page still contains publisher prose.
    Lint {
        #[arg(default_value = "fixtures")]
        roots: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum BundleCmd {
    /// Generate an Ed25519 signing key (JSON with key_id, seed, public). Keep it out of the repo.
    Keygen {
        #[arg(long)]
        key_id: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Build and sign a bundle for one channel into OUT (r/, s/, index.json).
    Build {
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        #[arg(long, default_value = "dev")]
        channel: String,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        out: PathBuf,
        /// Default: previous index sequence + 1 (or 1).
        #[arg(long)]
        sequence: Option<u64>,
    },
    /// Publish the recipes to a host: s3://bucket/prefix (R2, S3, MinIO) or a directory. Reads and verifies
    /// the live index, signs sequence+1, uploads new objects, then index.json, then reads everything back.
    Publish {
        #[arg(long = "recipes", default_value = "recipes")]
        recipes: Vec<PathBuf>,
        #[arg(long, default_value = "dev")]
        channel: String,
        #[arg(long)]
        key: PathBuf,
        #[command(flatten)]
        host: HostArgs,
        /// Oldest extension version that may apply this bundle (older clients keep their bundle).
        #[arg(long, default_value = "0.1.0")]
        min_client_version: String,
        /// Publish even when the recipes are unchanged (new sequence, same content).
        #[arg(long)]
        force: bool,
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete objects the live index has not referenced for --retention-days (state in gc-state.json).
    Gc {
        #[arg(long, default_value = "dev")]
        channel: String,
        /// Key file whose public key the live index must verify against (or use --trust).
        #[arg(long)]
        key: Option<PathBuf>,
        #[command(flatten)]
        host: HostArgs,
        #[arg(long, default_value_t = 30)]
        retention_days: u32,
        #[arg(long)]
        dry_run: bool,
    },
    /// Verify a bundle directory or a published channel URL against a public key (as a client would).
    Verify {
        #[arg(long, required_unless_present = "url", conflicts_with = "url")]
        dir: Option<PathBuf>,
        /// Channel base URL, e.g. https://recipes.example.com/rex-bundle/v1/dev/
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        key_id: String,
        /// Hex Ed25519 public key.
        #[arg(long)]
        public: String,
    },
}

#[derive(Args)]
struct HostArgs {
    /// s3://bucket[/prefix] or a local directory; `v1/<channel>/` is appended.
    #[arg(long)]
    to: String,
    /// S3 endpoint, e.g. https://<account>.r2.cloudflarestorage.com (default: $REX_S3_ENDPOINT).
    /// Credentials: REX_S3_ACCESS_KEY_ID/REX_S3_SECRET_ACCESS_KEY (or the AWS_* names).
    #[arg(long)]
    endpoint: Option<String>,
    /// SigV4 region; R2 uses `auto`.
    #[arg(long, env = "REX_S3_REGION", default_value = "auto")]
    region: String,
    /// Extra trusted keys (key_id:hex) for the live index, e.g. the previous key during a rotation.
    #[arg(long)]
    trust: Vec<String>,
}

#[derive(Args)]
struct PageArgs {
    #[arg(long = "recipes", default_value = "recipes")]
    recipes: Vec<PathBuf>,
    /// Recipe id to run (default: match the page).
    #[arg(long)]
    recipe: Option<String>,
    #[arg(long)]
    html: PathBuf,
    #[arg(long)]
    url: String,
    /// JSON object of host hints, e.g. {"title": "…"}.
    #[arg(long)]
    hints: Option<PathBuf>,
    /// Clock for relative dates and extract_time (RFC 3339); default: now.
    #[arg(long)]
    now: Option<String>,
    /// Print only the typed record.
    #[arg(long)]
    record_only: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Check {
            paths,
            fixtures,
            git_base,
        } => check(&paths, &fixtures, git_base.as_deref()),
        Command::Fixture(cmd) => fixture(cmd),
        Command::Extract(args) => extract(&args, false),
        Command::Explain(args) => extract(&args, true),
        Command::Match { recipes, url, html } => match_cmd(&recipes, &url, html.as_deref()),
        Command::Test {
            recipes,
            fixtures,
            update,
            filter,
        } => test(&recipes, &fixtures, update, &filter),
        Command::Bundle(cmd) => bundle(cmd),
        Command::Draft {
            from,
            url,
            schema,
            id,
            llm,
            model,
            recipes,
            out,
            also,
            negative,
            repair_rounds,
            url_host,
            url_include,
            keep_failing,
            inline_system,
            pack_only,
            force,
        } => draft_cmd(DraftArgs {
            from,
            url,
            schema,
            id,
            llm,
            model,
            recipes,
            out,
            also,
            negative,
            repair_rounds,
            url_host,
            url_include,
            keep_failing,
            inline_system,
            pack_only,
            force,
        }),
        Command::Gate {
            recipes,
            id,
            page,
            negative,
        } => gate_cmd(&recipes, &id, &page, &negative),
        Command::Fmt { file, json } => fmt_recipe(&file, json),
        Command::Readability { html, url } => readability_cmd(&html, &url),
        Command::Liveness {
            file,
            recipes,
            report,
            jobs,
            skip_datacenter_blocked,
        } => liveness(&file, &recipes, report.as_deref(), jobs, skip_datacenter_blocked),
    }
}

fn recipe_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            collect(p, &mut out)?;
        } else if p.is_file() {
            out.push(p.clone());
        } else {
            bail!("{} does not exist", p.display());
        }
    }
    out.sort();
    Ok(out)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "yaml" || x == "yml") {
            out.push(p);
        }
    }
    Ok(())
}

/// Recipes parsed from disk: the set, per-file parse errors, and the file each id came from.
struct Loaded {
    set: RecipeSet,
    errors: Vec<(PathBuf, String)>,
    origin: BTreeMap<String, PathBuf>,
}

/// Parse every recipe file into a set; collects per-file errors instead of stopping at the first.
fn load_set(paths: &[PathBuf]) -> Result<Loaded> {
    load_set_except(paths, None)
}

/// [`load_set`] without the file `skip` (the draft that `rex draft --force` replaces).
fn load_set_except(paths: &[PathBuf], skip: Option<&Path>) -> Result<Loaded> {
    let skip = skip.and_then(|p| p.canonicalize().ok());
    let registry = SchemaRegistry::embedded().context("loading embedded schemas")?;
    let mut loaded = Loaded {
        set: RecipeSet::new(registry),
        errors: Vec::new(),
        origin: BTreeMap::new(),
    };
    for f in recipe_files(paths)? {
        if skip.is_some() && f.canonicalize().ok() == skip {
            continue;
        }
        let text = std::fs::read_to_string(&f).with_context(|| format!("reading {}", f.display()))?;
        match loaded.set.add_yaml(&text) {
            Ok(id) => {
                loaded.origin.insert(id, f);
            }
            Err(e) => loaded.errors.push((f, e.message)),
        }
    }
    Ok(loaded)
}

fn engine(paths: &[PathBuf]) -> Result<Engine> {
    let Loaded { set, errors, .. } = load_set(paths)?;
    if let Some((f, e)) = errors.first() {
        bail!("{}: {e}", f.display());
    }
    Engine::new(set, Limits::default()).map_err(|errs| {
        anyhow!(
            "recipes do not compile:\n{}",
            errs.iter().map(|e| format!("  {e}")).collect::<Vec<_>>().join("\n")
        )
    })
}

fn check(paths: &[PathBuf], fixture_roots: &[PathBuf], git_base: Option<&str>) -> Result<ExitCode> {
    if let Some(base) = git_base {
        verify_git_base(base)?;
    }
    let Loaded {
        set,
        errors: parse_errors,
        origin,
    } = load_set(paths)?;
    let mut failed = false;
    for (f, e) in &parse_errors {
        println!("ERR {}: {e}", f.display());
        failed = true;
    }
    let limits = Limits::default();
    for id in set.ids() {
        let file = origin.get(id).map(|p| p.display().to_string()).unwrap_or_default();
        let mut problems: Vec<String> = Vec::new();
        let compiled = match set.compile(id, &limits) {
            Ok(c) => Some(c),
            Err(errs) => {
                problems.extend(errs.iter().map(ToString::to_string));
                None
            }
        };
        let base = set.is_base(id);
        if id.starts_with("generic/") && !base {
            problems.push(format!("{id}: generic recipes must not declare match.urls"));
        }
        if !id.starts_with("generic/") && base {
            problems.push(format!(
                "{id}: site recipes must declare match.urls (only generic/* recipes may omit it)"
            ));
        }
        if let (Some(base_ref), Some(path)) = (git_base, origin.get(id)) {
            if let Some(p) = version_gate(&set, id, path, base_ref)? {
                problems.push(p);
            }
        }
        if !fixture_roots.is_empty() && !base {
            let cases: usize = fixture_roots
                .iter()
                .map(|r| fixtures::discover(&r.join(id)).len())
                .sum();
            if cases < 2 {
                problems.push(format!(
                    "{id}: has {cases} fixture case(s) under {}, needs at least 2",
                    fixture_roots
                        .iter()
                        .map(|r| r.join(id).display().to_string())
                        .collect::<Vec<_>>()
                        .join(" + ")
                ));
            }
        }
        if problems.is_empty() {
            let c = compiled.expect("compiled when no problems");
            println!(
                "ok  {file}  ({id}@{} -> {}, {} rules)",
                c.reference.version,
                c.record.full_name(),
                c.rules.len()
            );
        } else {
            failed = true;
            for p in problems {
                println!("ERR {file}: {p}");
            }
        }
    }
    Ok(if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn parse_now(now: Option<&str>) -> Result<DateTime<Utc>> {
    match now {
        Some(s) => Ok(DateTime::parse_from_rfc3339(s)
            .with_context(|| format!("--now `{s}`"))?
            .with_timezone(&Utc)),
        None => Ok(Utc::now()),
    }
}

fn extract(args: &PageArgs, explain: bool) -> Result<ExitCode> {
    let engine = engine(&args.recipes)?;
    let html = std::fs::read_to_string(&args.html).with_context(|| format!("reading {}", args.html.display()))?;
    let doc = Document::parse(&html, &args.url, engine.limits())?;
    let hints: BTreeMap<String, String> = match &args.hints {
        Some(p) => {
            serde_json::from_str(&std::fs::read_to_string(p)?).with_context(|| format!("parsing {}", p.display()))?
        }
        None => BTreeMap::new(),
    };
    let now = parse_now(args.now.as_deref())?;
    let ctx = Ctx {
        now,
        fetch_time: Some(now),
        hints,
        source_id: String::new(),
        source_ref: String::new(),
        host: "cli".into(),
    };
    let (plan, result) = match &args.recipe {
        Some(id) => (
            None,
            engine
                .extract(id, &doc, &ctx)
                .ok_or_else(|| anyhow!("unknown recipe `{id}`"))?,
        ),
        None => {
            let (plan, r) = engine.extract_auto(&doc, &ctx);
            let r = r.ok_or_else(|| anyhow!("no recipe matched (detected types: {:?})", plan.detected_types))?;
            (Some(plan), r)
        }
    };
    if !explain {
        let json = if args.record_only {
            engine.record_json(&result.record)
        } else {
            engine.to_json(&result.extraction)
        };
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(p) = plan {
        println!(
            "match: url hits {:?}; confirmed {:?}; fallback {:?}; chosen {:?}",
            p.url_hits.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            p.confirmed,
            p.fallback,
            p.chosen
        );
    }
    let ext = &result.extraction;
    println!(
        "recipe {} -> {}  status {}  fuel {}",
        ext.recipe
            .as_ref()
            .map(|r| format!("{}@{}", r.id, r.version))
            .unwrap_or_default(),
        ext.schema,
        status_name(ext.status),
        result.fuel_used
    );
    for t in &result.trace {
        println!("\n{}", t.path);
        for a in &t.attempts {
            let mark = if a.outcome == "won" { "*" } else { " " };
            println!(
                "  {mark} [{}] {} ({}) -> {}",
                a.candidate, a.source, a.recipe, a.outcome
            );
            for r in &a.raw {
                println!("        raw: {r}");
            }
        }
        if t.value.is_empty() {
            println!("    = (missing)");
        } else {
            for v in &t.value {
                println!("    = {v}");
            }
        }
    }
    if !result.diagnostics.is_empty() {
        println!("\ndiagnostics:");
        for d in &result.diagnostics {
            println!("  {:?} {} {} {} {}", d.level, d.code, d.path, d.recipe, d.detail);
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn status_name(code: i32) -> &'static str {
    match code {
        1 => "OK",
        2 => "INVALID",
        3 => "FAILED_EXPECTATION",
        4 => "EMPTY",
        _ => "UNSPECIFIED",
    }
}

fn match_cmd(recipes: &[PathBuf], url: &str, html: Option<&Path>) -> Result<ExitCode> {
    let engine = engine(recipes)?;
    let parsed = url::Url::parse(url)?;
    println!("stage 1 (URL):");
    let hits = engine.match_url(&parsed);
    if hits.is_empty() {
        println!("  no site recipe matches the URL");
    }
    for (id, h) in &hits {
        println!(
            "  {id}  exact_host={} include_matched={} priority={}",
            h.exact_host, h.include_matched, h.priority
        );
    }
    if let Some(path) = html {
        let doc = Document::parse(&std::fs::read_to_string(path)?, url, engine.limits())?;
        let plan = engine.plan(&doc);
        println!("stage 2 (page): confirmed {:?}", plan.confirmed);
        println!("detected schema.org types: {:?}", plan.detected_types);
        println!("fallback: {:?}", plan.fallback);
        println!("chosen: {:?}", plan.chosen);
    }
    Ok(ExitCode::SUCCESS)
}

fn test(recipes: &[PathBuf], roots: &[PathBuf], update: bool, filter: &[String]) -> Result<ExitCode> {
    let Loaded { set, errors, .. } = load_set(recipes)?;
    if let Some((f, e)) = errors.first() {
        bail!("{}: {e}", f.display());
    }
    let limits = Limits::default();
    let shared =
        Engine::new(set.clone(), limits.clone()).map_err(|errs| anyhow!("recipes do not compile: {}", errs[0]))?;
    let roots: Vec<PathBuf> = if roots.is_empty() {
        ["fixtures", "conformance"]
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect()
    } else {
        roots.to_vec()
    };
    let (mut pass, mut fail, mut updated) = (0, 0, 0);
    for root in &roots {
        for dir in fixtures::discover(root) {
            let name = dir.display().to_string();
            if !filter.is_empty() && !filter.iter().any(|f| name.contains(f.as_str())) {
                continue;
            }
            match fixtures::run_case(&shared, &set, &limits, &dir, update) {
                Outcome::Pass => {
                    pass += 1;
                    println!("ok      {name}");
                }
                Outcome::Updated => {
                    updated += 1;
                    println!("updated {name}");
                }
                Outcome::Fail { diff } => {
                    fail += 1;
                    println!("FAIL    {name}\n{diff}");
                }
                Outcome::Error(e) => {
                    fail += 1;
                    println!("ERROR   {name}: {e}");
                }
            }
        }
    }
    println!("\n{pass} passed, {fail} failed, {updated} updated");
    Ok(if fail > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

#[derive(serde::Serialize, serde::Deserialize)]
struct KeyFile {
    key_id: String,
    seed: String,
    public: String,
}

fn read_signer(path: &Path) -> Result<rex_bundle::Signer25519> {
    let kf: KeyFile = serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading key file {}", path.display()))?,
    )
    .context("parsing key file")?;
    let seed = rex_bundle::parse_hex32(&kf.seed).ok_or_else(|| anyhow!("bad seed in key file"))?;
    Ok(rex_bundle::Signer25519::from_seed(&kf.key_id, seed))
}

fn bundle(cmd: BundleCmd) -> Result<ExitCode> {
    match cmd {
        BundleCmd::Keygen { key_id, out } => {
            if out.exists() {
                bail!("{} already exists; refusing to overwrite a signing key", out.display());
            }
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).map_err(|e| anyhow!("no OS randomness: {e}"))?;
            let signer = rex_bundle::Signer25519::from_seed(&key_id, seed);
            let file = KeyFile {
                key_id,
                seed: rex_bundle::hex(&seed),
                public: rex_bundle::hex(&signer.public_key()),
            };
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Created owner-only and exclusively: the seed is never readable by others, not even briefly, and a
            // key that appeared since the check above is not overwritten.
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut f = options
                .open(&out)
                .with_context(|| format!("creating {}", out.display()))?;
            std::io::Write::write_all(&mut f, serde_json::to_string_pretty(&file)?.as_bytes())?;
            println!(
                "wrote {} (key_id {}, public {})",
                out.display(),
                file.key_id,
                file.public
            );
            Ok(ExitCode::SUCCESS)
        }
        BundleCmd::Build {
            recipes,
            channel,
            key,
            out,
            sequence,
        } => {
            let signer = read_signer(&key)?;
            let Loaded { set, errors, .. } = load_set(&recipes)?;
            if let Some((f, e)) = errors.first() {
                bail!("{}: {e}", f.display());
            }
            Engine::new(set.clone(), Limits::default())
                .map_err(|errs| anyhow!("recipes do not compile: {}", errs[0]))?;
            let sequence = match sequence {
                Some(s) => s,
                None => {
                    let pinned = [rex_bundle::PinnedKey {
                        key_id: signer.key_id.clone(),
                        public: signer.public_key(),
                    }];
                    let existing = out.join("index.json");
                    match std::fs::read(&existing) {
                        // An index that does not verify with this key is not ours to continue from, and
                        // restarting at 1 would make every client reject the bundle as a rollback.
                        Ok(bytes) => rex_bundle::verify_index(set.registry().pool(), &bytes, &pinned)
                            .map(|i| i.sequence + 1)
                            .map_err(|e| {
                                anyhow!(
                                    "{} does not verify with {} ({e}); pass --sequence to choose one",
                                    existing.display(),
                                    signer.key_id
                                )
                            })?,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 1,
                        Err(e) => return Err(e).with_context(|| format!("reading {}", existing.display())),
                    }
                }
            };
            let now = Utc::now();
            let opts = rex_bundle::BuildOptions {
                channel,
                sequence,
                generated_at: prost_types::Timestamp {
                    seconds: now.timestamp(),
                    nanos: 0,
                },
                min_client_version: "0.1.0".into(),
            };
            let files = rex_bundle::build(&set, &opts, &signer)?;
            // Objects first (immutable; skip existing), index.json last.
            let (index, objects): (Vec<_>, Vec<_>) = files.into_iter().partition(|f| f.path == "index.json");
            let mut written = 0;
            for f in objects.iter().chain(index.iter()) {
                let p = out.join(&f.path);
                if f.path != "index.json" && p.exists() {
                    continue;
                }
                std::fs::create_dir_all(p.parent().expect("bundle paths have a parent"))?;
                std::fs::write(&p, &f.bytes)?;
                written += 1;
            }
            println!(
                "bundle {} sequence {} -> {} ({} files written)",
                opts.channel,
                sequence,
                out.display(),
                written
            );
            Ok(ExitCode::SUCCESS)
        }
        BundleCmd::Publish {
            recipes,
            channel,
            key,
            host,
            min_client_version,
            force,
            dry_run,
        } => {
            let signer = read_signer(&key)?;
            let mut trusted = vec![rex_bundle::PinnedKey {
                key_id: signer.key_id.clone(),
                public: signer.public_key(),
            }];
            for t in &host.trust {
                trusted.push(publish::parse_trust(t)?);
            }
            let Loaded { set, errors, .. } = load_set(&recipes)?;
            if let Some((f, e)) = errors.first() {
                bail!("{}: {e}", f.display());
            }
            Engine::new(set.clone(), Limits::default())
                .map_err(|errs| anyhow!("recipes do not compile: {}", errs[0]))?;
            let store = publish::open_store(&host.to, &channel, host.endpoint.as_deref(), &host.region)?;
            let opts = publish::PublishOptions {
                channel: &channel,
                now: Utc::now(),
                force,
                dry_run,
                min_client_version: &min_client_version,
            };
            let report = publish::publish(store.as_ref(), &set, &signer, &trusted, &opts)?;
            let prev = report.previous.map_or("none".to_string(), |p| p.to_string());
            for path in &report.repaired {
                println!(
                    "{} {path} (missing or corrupt on the host)",
                    if dry_run { "would repair" } else { "repaired" }
                );
            }
            match report.sequence {
                None => println!(
                    "{}: unchanged at sequence {prev}; nothing published (use --force to re-sign)",
                    store.describe()
                ),
                Some(seq) => {
                    for path in &report.uploaded {
                        println!("{} {path}", if dry_run { "would put" } else { "put" });
                    }
                    println!(
                        "{}{}: sequence {prev} -> {seq}; {} uploaded, {} already present",
                        if dry_run { "dry run: " } else { "" },
                        store.describe(),
                        report.uploaded.len(),
                        report.skipped
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        BundleCmd::Gc {
            channel,
            key,
            host,
            retention_days,
            dry_run,
        } => {
            let mut trusted = Vec::new();
            if let Some(key) = &key {
                let signer = read_signer(key)?;
                trusted.push(rex_bundle::PinnedKey {
                    key_id: signer.key_id.clone(),
                    public: signer.public_key(),
                });
            }
            for t in &host.trust {
                trusted.push(publish::parse_trust(t)?);
            }
            if trusted.is_empty() {
                bail!("gc needs --key or --trust: it only deletes against an index it can verify");
            }
            let store = publish::open_store(&host.to, &channel, host.endpoint.as_deref(), &host.region)?;
            let registry = SchemaRegistry::embedded()?;
            let report = publish::gc(
                store.as_ref(),
                registry.pool(),
                &trusted,
                Utc::now(),
                chrono::Duration::days(retention_days.into()),
                dry_run,
            )?;
            for p in &report.newly_unreferenced {
                println!("marked  {p}");
            }
            for p in &report.deleted {
                println!("{} {p}", if dry_run { "would delete" } else { "deleted" });
            }
            println!(
                "{}{}: sequence {}, {} referenced, {} waiting out the {retention_days}-day retention, {} deleted",
                if dry_run { "dry run: " } else { "" },
                store.describe(),
                report.sequence,
                report.referenced,
                report.waiting.len(),
                report.deleted.len()
            );
            Ok(ExitCode::SUCCESS)
        }
        BundleCmd::Verify {
            dir,
            url,
            key_id,
            public,
        } => {
            let key = rex_bundle::parse_hex32(&public).ok_or_else(|| anyhow!("--public must be 64 hex chars"))?;
            let registry = SchemaRegistry::embedded()?;
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(30)))
                .build()
                .into();
            let read = |path: &str| -> Result<Vec<u8>> {
                match (&dir, &url) {
                    (Some(dir), _) => Ok(std::fs::read(dir.join(path))?),
                    (None, Some(base)) => {
                        let u = url::Url::parse(&format!("{}/", base.trim_end_matches('/')))?.join(path)?;
                        Ok(agent
                            .get(u.as_str())
                            .call()
                            .with_context(|| format!("GET {u}"))?
                            .body_mut()
                            .with_config()
                            .limit(64 << 20)
                            .read_to_vec()?)
                    }
                    (None, None) => unreachable!("clap requires --dir or --url"),
                }
            };
            let index = rex_bundle::verify_index(
                registry.pool(),
                &read("index.json")?,
                &[rex_bundle::PinnedKey { key_id, public: key }],
            )?;
            let mut bad = 0;
            for e in &index.entries {
                let r = e.r#ref.as_ref().ok_or_else(|| anyhow!("entry without ref"))?;
                let bytes = read(&format!("r/{}.json", r.sha256))?;
                let ok = rex_bundle::sha256_hex(&bytes) == r.sha256;
                if !ok {
                    bad += 1;
                }
                println!(
                    "{} {}@{} {}",
                    if ok { "ok " } else { "BAD" },
                    r.id,
                    r.version,
                    if e.base { "(base)" } else { "" }
                );
            }
            let schemas = index.schemas.as_ref().ok_or_else(|| anyhow!("index without schemas"))?;
            let fds_ok = rex_bundle::sha256_hex(&read(&format!("s/{}.fds", schemas.sha256))?) == schemas.sha256;
            println!(
                "{} schemas {} ({} messages)",
                if fds_ok { "ok " } else { "BAD" },
                schemas.sha256,
                schemas.messages.len()
            );
            println!("channel {} sequence {}", index.channel, index.sequence);
            Ok(if bad == 0 && fds_ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

struct DraftArgs {
    from: PathBuf,
    url: Option<String>,
    schema: String,
    id: Option<String>,
    llm: Option<String>,
    model: String,
    recipes: Vec<PathBuf>,
    out: PathBuf,
    also: Vec<PathBuf>,
    negative: Vec<PathBuf>,
    repair_rounds: usize,
    url_host: Vec<String>,
    url_include: Vec<String>,
    keep_failing: bool,
    inline_system: bool,
    pack_only: bool,
    force: bool,
}

/// A captured page for the gate: a fixture directory (page.html + meta.json).
fn gate_page(dir: &Path) -> Result<gate::Page> {
    let meta: rex_engine::fixtures::FixtureMeta = serde_json::from_str(
        &std::fs::read_to_string(dir.join("meta.json")).with_context(|| format!("{}/meta.json", dir.display()))?,
    )?;
    Ok(gate::Page {
        label: dir.display().to_string(),
        url: meta.url,
        html: fixtures::read_page(dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?,
    })
}

fn gate_cmd(recipes: &[PathBuf], id: &str, pages: &[PathBuf], negatives: &[PathBuf]) -> Result<ExitCode> {
    let engine = engine(recipes)?;
    let schema = engine
        .recipes()
        .get(id)
        .map(|r| r.schema.clone())
        .ok_or_else(|| anyhow!("no recipe `{id}`"))?;
    let positives = pages.iter().map(|p| gate_page(p)).collect::<Result<Vec<_>>>()?;
    let negatives = negatives.iter().map(|p| gate_page(p)).collect::<Result<Vec<_>>>()?;
    let report = gate::check(&engine, id, &schema, &positives, &negatives, Utc::now());
    println!("{}", serde_json::to_string(&report)?);
    Ok(if report.pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn draft_cmd(a: DraftArgs) -> Result<ExitCode> {
    let (html_path, meta_url) = if a.from.is_dir() {
        let meta: rex_engine::fixtures::FixtureMeta =
            serde_json::from_str(&std::fs::read_to_string(a.from.join("meta.json"))?)
                .with_context(|| format!("{}/meta.json", a.from.display()))?;
        (a.from.join("page.html"), Some(meta.url))
    } else {
        (a.from.clone(), None)
    };
    let url = a
        .url
        .or(meta_url)
        .ok_or_else(|| anyhow!("--url is required when --from is an HTML file"))?;
    let html = if a.from.is_dir() {
        fixtures::read_page(&a.from).map_err(|e| anyhow!("{}: {e}", a.from.display()))?
    } else {
        std::fs::read_to_string(&html_path).with_context(|| format!("reading {}", html_path.display()))?
    };
    let id = match a.id {
        Some(id) => id,
        None => {
            let host = url::Url::parse(&url)?
                .host_str()
                .unwrap_or("site")
                .trim_start_matches("www.")
                .to_string();
            let short = a.schema.rsplit('.').next().unwrap_or("record").to_ascii_lowercase();
            format!("{host}/{short}")
        }
    };
    let path = a.out.join(format!("{id}.yaml"));
    if path.exists() && !a.force && !a.pack_only {
        bail!("{} exists; pass --force to replace it", path.display());
    }
    // The draft being replaced must not count as an existing recipe with the same id.
    let Loaded { set, errors, origin } = load_set_except(&a.recipes, a.force.then_some(path.as_path()))?;
    if let Some((f, e)) = errors.first() {
        bail!("{}: {e}", f.display());
    }
    // Only reviewed recipes are shown to the model as the base and the example: not other drafts, and not
    // auto-harvested recipes that no one has looked at.
    let reviewed: BTreeSet<String> = origin
        .iter()
        .filter(|(_, f)| {
            !f.components()
                .any(|c| c.as_os_str() == "drafts" || c.as_os_str() == "auto")
        })
        .map(|(id, _)| id.clone())
        .collect();
    let source_label = a.from.display().to_string();
    let also = a.also.iter().map(|p| gate_page(p)).collect::<Result<Vec<_>>>()?;
    let negatives = a.negative.iter().map(|p| gate_page(p)).collect::<Result<Vec<_>>>()?;
    let req = draft::DraftRequest {
        id: &id,
        schema: &a.schema,
        url: &url,
        html: &html,
        source_label: &source_label,
        now: Utc::now(),
        also: &also,
        negatives: &negatives,
        repair_rounds: a.repair_rounds,
        urls: (!a.url_host.is_empty()).then(|| draft::UrlRules {
            hosts: a.url_host.clone(),
            include: a.url_include.clone(),
        }),
        reviewed: Some(&reviewed),
    };
    if a.pack_only {
        println!("{}", draft::pack_for(&set, &req)?);
        return Ok(ExitCode::SUCCESS);
    }
    let api_key = std::env::var("REX_LLM_API_KEY").ok().filter(|k| !k.is_empty());
    let endpoint = a.llm.ok_or_else(|| anyhow!("--llm is required"))?;
    let llm = draft::HttpLlm::new(
        &endpoint,
        draft::Api::detect(&endpoint),
        &a.model,
        api_key,
        a.inline_system,
    );
    let d = draft::draft(&llm, &set, &req)?;
    if !d.problems.is_empty() {
        println!(
            "quality gate FAILED after {} repair round(s):\n  {}",
            d.repairs,
            d.problems.join("\n  ")
        );
        if !a.keep_failing {
            return Ok(ExitCode::FAILURE);
        }
    }
    std::fs::create_dir_all(path.parent().expect("draft path has a parent"))?;
    std::fs::write(&path, &d.yaml)?;
    println!(
        "wrote {} after {} attempt(s), {} repair round(s); quality gate {}",
        path.display(),
        d.attempts,
        d.repairs,
        if d.problems.is_empty() { "passed" } else { "FAILED" }
    );
    println!(
        "on {url}: {} {}; fields: {}{}",
        if d.confirmed {
            "matched,"
        } else {
            "NOT matched by its own match rules,"
        },
        d.status,
        d.filled.join(", "),
        if d.missing_required.is_empty() {
            String::new()
        } else {
            format!("; missing required: {}", d.missing_required.join(", "))
        }
    );
    println!("next: capture >= 2 fixtures, `rex test --update`, review, then move it to recipes/sites/");
    Ok(ExitCode::SUCCESS)
}

fn fmt_recipe(file: &Path, json: bool) -> Result<ExitCode> {
    let registry = SchemaRegistry::embedded()?;
    let recipe = rex_engine::recipe::parse_recipe(&std::fs::read_to_string(file)?, &registry)
        .map_err(|e| anyhow!("{}: {e}", file.display()))?;
    let (canonical, sha) = rex_engine::recipe::canonical_json(&recipe, &registry).map_err(|e| anyhow!(e))?;
    if json {
        println!("{canonical}");
    } else {
        let value: serde_json::Value = serde_json::from_str(&canonical)?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        eprintln!("sha256 {sha}");
    }
    Ok(ExitCode::SUCCESS)
}

fn readability_cmd(html: &Path, url: &str) -> Result<ExitCode> {
    let limits = Limits::default();
    let doc = Document::parse(&std::fs::read_to_string(html)?, url, &limits)?;
    let r = rex_readability::extract(&doc, &rex_dom::Budget::new(limits.fuel)).map_err(|e| anyhow!("{e}"))?;
    let out = ReadableJson {
        title: r.title,
        byline: r.byline,
        excerpt: r.excerpt,
        content_text: r.content_text,
    };
    println!("{}", serde_json::to_string(&out)?);
    Ok(ExitCode::SUCCESS)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadableJson {
    title: String,
    byline: String,
    excerpt: String,
    content_text: String,
}

/// FR-RC-4: compare the recipe with its version at `base_ref` (if the file existed there).
/// The base ref must name a commit; an unknown ref (a force-pushed-away `before`, a missing fetch) or running
/// outside a work tree must fail the check, never read as "every recipe is new".
fn verify_git_base(base_ref: &str) -> Result<()> {
    let ok = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &format!("{base_ref}^{{commit}}")])
        .output()?;
    if !ok.status.success() {
        bail!("--git-base `{base_ref}` is not a commit in this repository (fetch it, e.g. with fetch-depth: 0)");
    }
    Ok(())
}

fn version_gate(set: &RecipeSet, id: &str, path: &Path, base_ref: &str) -> Result<Option<String>> {
    let top = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    if !top.status.success() {
        bail!(
            "the version gate needs a git work tree: {}",
            String::from_utf8_lossy(&top.stderr).trim()
        );
    }
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    let abs = std::fs::canonicalize(path)?;
    let rel = abs
        .strip_prefix(&top)
        .map_err(|_| anyhow!("{} is outside the git work tree", path.display()))?;
    // Absent at the base (a new recipe) is the only case that skips the gate; any other git failure is fatal.
    let listed = std::process::Command::new("git")
        .args(["ls-tree", "--name-only", base_ref, "--"])
        .arg(rel)
        .current_dir(&top)
        .output()?;
    if !listed.status.success() {
        bail!(
            "git ls-tree {base_ref}: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        );
    }
    if listed.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(None); // new file
    }
    let old = std::process::Command::new("git")
        .arg("show")
        .arg(format!("{base_ref}:{}", rel.display()))
        .current_dir(&top)
        .output()?;
    if !old.status.success() {
        bail!(
            "git show {base_ref}:{}: {}",
            rel.display(),
            String::from_utf8_lossy(&old.stderr).trim()
        );
    }
    let registry = set.registry();
    let old_recipe = rex_engine::recipe::parse_recipe(&String::from_utf8_lossy(&old.stdout), registry)
        .map_err(|e| anyhow!("{base_ref}:{}: {e}", rel.display()))?;
    let new_recipe = set.get(id).expect("loaded recipe");
    let (_, old_sha) = rex_engine::recipe::canonical_json(&old_recipe, registry).map_err(|e| anyhow!(e))?;
    let (_, new_sha) = rex_engine::recipe::canonical_json(new_recipe, registry).map_err(|e| anyhow!(e))?;
    // Compare content without the version field itself.
    let strip = |r: &rex_engine::recipe::RecipeProto| {
        let mut c = r.clone();
        c.version = 0;
        rex_engine::recipe::canonical_json(&c, registry).map(|(_, sha)| sha)
    };
    let changed = strip(&old_recipe).map_err(|e| anyhow!(e))? != strip(new_recipe).map_err(|e| anyhow!(e))?;
    let _ = (old_sha, new_sha);
    Ok(match (changed, new_recipe.version) {
        (true, v) if v != old_recipe.version + 1 => Some(format!(
            "{id}: content changed since {base_ref} but version is {v}; it must be {}",
            old_recipe.version + 1
        )),
        (false, v) if v != old_recipe.version => Some(format!(
            "{id}: version changed ({} -> {v}) without a content change since {base_ref}",
            old_recipe.version
        )),
        _ => None,
    })
}

fn fixture(cmd: FixtureCmd) -> Result<ExitCode> {
    match cmd {
        FixtureCmd::Capture { url, id, case, root } => {
            let (final_url, html) = fetch_page(&url)?;
            let dir = root.join(&id).join(&case);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("page.html"), &html)?;
            let meta = serde_json::json!({ "url": final_url, "fetch_time": Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true) });
            std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)? + "\n")?;
            println!("captured {} bytes -> {}", html.len(), dir.display());
            Ok(ExitCode::SUCCESS)
        }
        FixtureCmd::Redact { from, to, gzip } => {
            let html = fixtures::read_page(&from).map_err(|e| anyhow!("{}: {e}", from.display()))?;
            let redacted = rex_redact::redact_html(&html);
            let violations = rex_redact::lint_html(&redacted);
            if !violations.is_empty() {
                bail!("redaction left prose behind:\n  {}", violations.join("\n  "));
            }
            std::fs::create_dir_all(&to)?;
            if gzip {
                use std::io::Write;
                let mut gz = flate2::write::GzEncoder::new(
                    std::fs::File::create(to.join("page.html.gz"))?,
                    flate2::Compression::best(),
                );
                gz.write_all(redacted.as_bytes())?;
                gz.finish()?;
            } else {
                std::fs::write(to.join("page.html"), &redacted)?;
            }
            std::fs::copy(from.join("meta.json"), to.join("meta.json"))?;
            println!(
                "redacted {} -> {} ({} -> {} bytes); now run `rex test --update {}` and review expected.json",
                from.display(),
                to.display(),
                html.len(),
                redacted.len(),
                to.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        FixtureCmd::Import {
            file,
            root,
            recipe,
            case,
        } => {
            let f: FixtureFile = serde_json::from_str(&std::fs::read_to_string(&file)?)
                .with_context(|| format!("{} is not a rex fixture file", file.display()))?;
            if f.format != "rex.fixture.v1" {
                bail!("unsupported fixture format `{}`", f.format);
            }
            let recipe = recipe
                .or(f.recipe)
                .ok_or_else(|| anyhow!("the file names no recipe; pass --recipe"))?;
            if recipe.contains("..") || recipe.starts_with('/') {
                bail!("bad recipe id `{recipe}`");
            }
            let case = match case {
                Some(c) => c,
                None => case_name(&f.url)?,
            };
            let violations = rex_redact::lint_html(&f.page_html);
            if !violations.is_empty() {
                bail!("the page is not redacted:\n  {}", violations.join("\n  "));
            }
            DateTime::parse_from_rfc3339(&f.fetch_time).context("fetchTime must be RFC 3339")?;
            let dir = root.join(&recipe).join(&case);
            if dir.exists() {
                bail!("{} already exists; pass --case to choose another name", dir.display());
            }
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("page.html"), &f.page_html)?;
            let meta = rex_engine::fixtures::FixtureMeta {
                url: f.url,
                fetch_time: f.fetch_time,
                recipe: None,
                hints: BTreeMap::new(),
                synthetic: false,
            };
            std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(&meta)? + "\n")?;
            println!(
                "imported {} -> {}; now run `rex test --update {}` and review expected.json",
                file.display(),
                dir.display(),
                dir.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        FixtureCmd::Lint { roots } => {
            let mut bad = 0;
            let mut checked = 0;
            for root in &roots {
                for dir in fixtures::discover(root) {
                    let meta: rex_engine::fixtures::FixtureMeta =
                        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json"))?)
                            .with_context(|| format!("{}/meta.json", dir.display()))?;
                    if meta.synthetic {
                        continue;
                    }
                    checked += 1;
                    let v = rex_redact::lint_html(
                        &fixtures::read_page(&dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?,
                    );
                    if !v.is_empty() {
                        bad += 1;
                        println!("UNREDACTED {}:\n  {}", dir.display(), v.join("\n  "));
                    }
                }
            }
            println!("{checked} captured fixture(s) checked, {bad} with unredacted prose");
            Ok(if bad == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE })
        }
    }
}

/// What the extension's "Save as fixture" downloads.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureFile {
    format: String,
    recipe: Option<String>,
    url: String,
    fetch_time: String,
    page_html: String,
}

/// A case name from a URL: the last non-empty path segment, slugged (or the host for a root URL).
fn case_name(url: &str) -> Result<String> {
    let u = url::Url::parse(url).with_context(|| format!("bad url `{url}`"))?;
    let seg = u
        .path_segments()
        .and_then(|mut s| s.rfind(|x| !x.is_empty()).map(str::to_string))
        .unwrap_or_else(|| u.host_str().unwrap_or("page").to_string());
    let seg = seg.trim_end_matches(".html").trim_end_matches(".htm");
    let slug: String = seg
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    Ok(if slug.is_empty() {
        "page".to_string()
    } else {
        slug.chars().take(60).collect()
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LivenessFile {
    checks: Vec<LivenessCheck>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LivenessCheck {
    recipe: String,
    urls: Vec<String>,
    /// Why the site refuses datacenter IPs (bot wall, HTTP 403/412). Checked from residential lines only;
    /// `--skip-datacenter-blocked` reports it as skipped.
    #[serde(default)]
    datacenter_blocked: Option<String>,
}

#[derive(serde::Serialize)]
struct LivenessResult {
    recipe: String,
    url: String,
    ok: bool,
    /// Not fetched (`datacenter_blocked` under `--skip-datacenter-blocked`); `problem` says why.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    skipped: bool,
    matched: Option<String>,
    status: Option<String>,
    problem: Option<String>,
}

/// The page's `<title>`, trimmed to 80 characters ("" when it has none).
fn page_title(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    let sel = scraper::Selector::parse("title").expect("static selector");
    doc.select(&sel)
        .next()
        .map(|t| {
            t.text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect()
}

/// How long to wait before retrying after a 429/503: the `Retry-After` seconds (an HTTP date or nothing
/// means 5 s), at most 30 s.
fn retry_after(header: Option<&str>) -> std::time::Duration {
    let secs = header.and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(5);
    std::time::Duration::from_secs(secs.min(30))
}

/// URLs grouped by host, in first-seen order: one worker fetches a host's URLs one after another, so a
/// site never gets parallel requests from a liveness run (HN answers those with 429).
fn by_host(work: &[(String, String)]) -> Vec<Vec<(String, String)>> {
    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for (recipe, url) in work {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        match groups.iter_mut().find(|(h, _)| *h == host) {
            Some((_, g)) => g.push((recipe.clone(), url.clone())),
            None => groups.push((host, vec![(recipe.clone(), url.clone())])),
        }
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

fn fetch_page(url: &str) -> Result<(String, String)> {
    // IPv4 only: ureq tries the first resolved address without falling back, and the hosts this runs on
    // (GitHub runners, the workstation) have no IPv6 route, so an AAAA-first site hangs until the timeout.
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .ip_family(ureq::config::IpFamily::Ipv4Only)
        .http_status_as_error(false)
        .build()
        .into();
    let get = || {
        agent
            .get(url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0 Safari/537.36",
            )
            .header("Accept-Language", "en,zh;q=0.8")
            .call()
            .with_context(|| format!("GET {url}"))
    };
    let mut res = get()?;
    // Rate limited or briefly unavailable: wait as asked (capped) and try once more.
    if matches!(res.status().as_u16(), 429 | 503) {
        let wait = retry_after(res.headers().get("retry-after").and_then(|v| v.to_str().ok()));
        std::thread::sleep(wait);
        res = get()?;
    }
    let status = res.status().as_u16();
    if status >= 400 {
        bail!("GET {url}: http status: {status}");
    }
    let final_url = res.get_uri().to_string();
    let content_type = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = res.into_body().with_config().limit(10 * 1024 * 1024).read_to_vec()?;
    // Some CDNs answer clients they refuse with an empty 200; say so rather than "no recipe matched".
    if bytes.is_empty() {
        bail!("GET {url}: empty response body (from {final_url})");
    }
    Ok((final_url, decode_html(&content_type, &bytes)))
}

/// Decode an HTML body: the charset from Content-Type, else from `<meta charset>` /
/// `<meta http-equiv content="...; charset=...">` in the first 4 KB, else UTF-8 (lossy). Many Chinese sites
/// are GBK or GB2312.
fn decode_html(content_type: &str, bytes: &[u8]) -> String {
    // The header's charset parameter (names are case-insensitive), else the page's own <meta> declaration;
    // a label neither side recognizes falls through to the next source, then to UTF-8.
    let from_header = content_type.split(';').find_map(|p| {
        let (name, value) = p.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches(['"', '\'']).to_string())
    });
    let from_meta = || {
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).to_ascii_lowercase();
        let at = head.find("charset=")?;
        let rest = head[at + 8..].trim_start_matches(['"', '\'']);
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    };
    let known = |l: Option<String>| l.and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()));
    let encoding = known(from_header)
        .or_else(|| known(from_meta()))
        .unwrap_or(encoding_rs::UTF_8);
    let (text, _, _) = encoding.decode(bytes);
    text.into_owned()
}

fn liveness(
    file: &Path,
    recipes: &[PathBuf],
    report: Option<&Path>,
    jobs: usize,
    skip_datacenter_blocked: bool,
) -> Result<ExitCode> {
    let spec: LivenessFile =
        serde_yaml::from_str(&std::fs::read_to_string(file)?).with_context(|| format!("parsing {}", file.display()))?;
    let engine = engine(recipes)?;
    let now = Utc::now();
    let ctx = Ctx {
        now,
        fetch_time: Some(now),
        hints: BTreeMap::new(),
        source_id: String::new(),
        source_ref: String::new(),
        host: "liveness".into(),
    };
    for check in &spec.checks {
        if engine.compiled(&check.recipe).is_none() {
            bail!("{} names unknown recipe `{}`", file.display(), check.recipe);
        }
    }
    let mut results = Vec::new();
    let mut work: Vec<(String, String)> = Vec::new();
    for c in &spec.checks {
        for u in &c.urls {
            match (&c.datacenter_blocked, skip_datacenter_blocked) {
                (Some(why), true) => {
                    println!("skip {} {u} — datacenter IPs refused: {why}", c.recipe);
                    results.push(LivenessResult {
                        recipe: c.recipe.clone(),
                        url: u.clone(),
                        ok: true,
                        skipped: true,
                        matched: None,
                        status: None,
                        problem: Some(format!("skipped: datacenter IPs refused ({why})")),
                    });
                }
                _ => work.push((c.recipe.clone(), u.clone())),
            }
        }
    }
    let groups = by_host(&work);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (tx, rx) = std::sync::mpsc::channel::<(String, String, Result<(String, String)>)>();
    std::thread::scope(|scope| {
        for _ in 0..jobs.max(1) {
            let tx = tx.clone();
            let (groups, next) = (&groups, &next);
            scope.spawn(move || loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(group) = groups.get(i) else { break };
                for (n, (recipe, url)) in group.iter().enumerate() {
                    if n > 0 {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                    if tx.send((recipe.clone(), url.clone(), fetch_page(url))).is_err() {
                        return;
                    }
                }
            });
        }
        drop(tx);
        for (recipe, url, fetched) in rx {
            let mut r = LivenessResult {
                recipe,
                url,
                ok: false,
                skipped: false,
                matched: None,
                status: None,
                problem: None,
            };
            let check_recipe = r.recipe.clone();
            // What the page was, for telling a bot wall or an interstitial from a changed page.
            let mut seen = String::new();
            match fetched.and_then(|(final_url, html)| {
                seen = format!("{} bytes from {final_url}, title {:?}", html.len(), page_title(&html));
                Ok(Document::parse(&html, &final_url, engine.limits())?)
            }) {
                Err(e) => r.problem = Some(format!("{e:#}")),
                Ok(doc) => {
                    let (plan, result) = engine.extract_auto(&doc, &ctx);
                    r.matched = plan.chosen.clone();
                    match result {
                        None => r.problem = Some(format!("no recipe matched ({seen})")),
                        Some(res) => {
                            let status = status_name(res.extraction.status).to_string();
                            r.ok = plan.chosen.as_deref() == Some(check_recipe.as_str()) && status == "OK";
                            if !r.ok {
                                let mut why = Vec::new();
                                if plan.chosen.as_deref() != Some(check_recipe.as_str()) {
                                    why.push(format!("matched {:?} instead", plan.chosen));
                                }
                                if status != "OK" {
                                    why.push(format!(
                                        "status {status}; expectations failed {:?}; missing {:?}",
                                        res.extraction.expectations_failed, res.extraction.missing_required
                                    ));
                                }
                                r.problem = Some(why.join("; "));
                            }
                            r.status = Some(status);
                        }
                    }
                }
            }
            println!(
                "{} {} {}{}",
                if r.ok { "ok  " } else { "FAIL" },
                r.recipe,
                r.url,
                r.problem.as_ref().map(|p| format!(" — {p}")).unwrap_or_default()
            );
            results.push(r);
        }
    });
    results.sort_by(|a, b| (&a.recipe, &a.url).cmp(&(&b.recipe, &b.url)));
    if let Some(path) = report {
        std::fs::write(path, serde_json::to_string_pretty(&results)?)?;
    }
    let failed = results.iter().filter(|r| !r.ok).count();
    let skipped = results.iter().filter(|r| r.skipped).count();
    println!(
        "\n{} checked, {failed} failing{}",
        results.len() - skipped,
        if skipped > 0 {
            format!(", {skipped} skipped")
        } else {
            String::new()
        }
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

#[cfg(test)]
mod liveness_tests {
    use super::{by_host, page_title, retry_after, LivenessFile};

    #[test]
    fn urls_of_one_host_go_to_one_group_in_order() {
        // Arrange
        let work: Vec<(String, String)> = [
            ("hn/front", "https://news.ycombinator.com/"),
            ("lobsters", "https://lobste.rs/"),
            ("hn/front", "https://news.ycombinator.com/newest"),
            ("hn/item", "https://news.ycombinator.com/item?id=1"),
        ]
        .iter()
        .map(|(r, u)| (r.to_string(), u.to_string()))
        .collect();

        // Act
        let groups = by_host(&work);

        // Assert
        let urls: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.iter().map(|(_, u)| u.as_str()).collect())
            .collect();
        assert_eq!(
            urls,
            vec![
                vec![
                    "https://news.ycombinator.com/",
                    "https://news.ycombinator.com/newest",
                    "https://news.ycombinator.com/item?id=1"
                ],
                vec!["https://lobste.rs/"],
            ]
        );
    }

    #[test]
    fn retry_after_takes_seconds_capped_at_30_and_defaults_to_5() {
        // Arrange
        let cases = [
            (Some("2"), 2),
            (Some("120"), 30),
            (Some("Wed, 21 Oct 2026 07:28:00 GMT"), 5),
            (None, 5),
        ];

        // Act
        let got: Vec<u64> = cases.iter().map(|(h, _)| retry_after(*h).as_secs()).collect();

        // Assert
        assert_eq!(got, cases.iter().map(|(_, s)| *s).collect::<Vec<u64>>());
    }

    #[test]
    fn page_title_is_collapsed_and_cut_to_80_characters() {
        // Arrange
        let html = format!(
            "<html><head><title>\n  Robot   check {}</title></head></html>",
            "x".repeat(100)
        );

        // Act
        let title = page_title(&html);

        // Assert
        assert!(title.starts_with("Robot check xxx"), "{title}");
        assert_eq!(title.chars().count(), 80);
    }

    #[test]
    fn a_check_may_say_why_datacenter_ips_are_refused() {
        // Arrange
        let yaml = "checks:\n  - recipe: a.com/x\n    urls: [\"https://a.com/1\"]\n    datacenter_blocked: \"HTTP 403\"\n  - recipe: b.com/y\n    urls: [\"https://b.com/1\"]\n";

        // Act
        let spec: LivenessFile = serde_yaml::from_str(yaml).unwrap();

        // Assert
        assert_eq!(spec.checks[0].datacenter_blocked.as_deref(), Some("HTTP 403"));
        assert_eq!(spec.checks[1].datacenter_blocked, None);
    }

    #[test]
    fn an_unknown_key_in_a_check_is_an_error() {
        // Arrange
        let yaml = "checks:\n  - recipe: a.com/x\n    urls: []\n    blocked: true\n";

        // Act
        let parsed = serde_yaml::from_str::<LivenessFile>(yaml);

        // Assert
        assert!(parsed.is_err());
    }
}

#[cfg(test)]
mod decode_tests {
    use super::decode_html;

    #[test]
    fn gbk_page_is_decoded_from_its_meta_charset() {
        // Arrange
        let (gbk, _, _) =
            encoding_rs::GBK.encode("<html><head><meta charset=\"gbk\"></head><body>中关村在线</body></html>");

        // Act
        let text = decode_html("text/html", &gbk);

        // Assert
        assert!(text.contains("中关村在线"), "{text}");
    }

    #[test]
    fn header_charset_wins_over_meta() {
        // Arrange
        let (gb, _, _) = encoding_rs::GBK.encode("<meta charset=\"utf-8\">新闻");

        // Act
        let text = decode_html("text/html; charset=GB2312", &gb);

        // Assert
        assert!(text.contains("新闻"), "{text}");
    }

    #[test]
    fn header_charset_name_is_case_insensitive() {
        // Arrange
        let (gb, _, _) = encoding_rs::GBK.encode("新闻");

        // Act
        let text = decode_html("text/html; Charset=\"GBK\"", &gb);

        // Assert
        assert_eq!(text, "新闻");
    }

    #[test]
    fn an_unknown_header_charset_falls_back_to_the_meta_declaration() {
        // Arrange
        let (gb, _, _) = encoding_rs::GBK.encode("<meta charset=\"gbk\">新闻");

        // Act
        let text = decode_html("text/html; charset=x-unknown", &gb);

        // Assert
        assert!(text.contains("新闻"), "{text}");
    }
}
