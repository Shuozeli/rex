//! Publishing bundles to a host (design.md §6, decisions.md U16): `rex bundle publish` and `rex bundle gc`.
//!
//! A [`Store`] is one channel root (`<base>/v1/<channel>/`). Two kinds exist: a local directory and an
//! S3-compatible bucket (Cloudflare R2 by default; MinIO and AWS S3 work the same way).
//!
//! Publish order is what keeps clients safe mid-upload: content-addressed objects first (immutable, never
//! overwritten), `index.json` last, then a read-back that must verify. Deletion only happens in `gc`, and only
//! for objects the current *verified* index does not reference and that have stayed unreferenced for the
//! retention period (recorded in `gc-state.json`), so a client holding the previous index can still fetch.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use prost_reflect::DescriptorPool;
use rex_bundle::{BuildOptions, PinnedKey, Signer25519};
use rex_engine::RecipeSet;
use rex_proto::rex::bundle::v1 as pb;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

pub const INDEX: &str = "index.json";
pub const GC_STATE: &str = "gc-state.json";
pub const OBJECT_CACHE: &str = "public, max-age=31536000, immutable";
pub const INDEX_CACHE: &str = "public, max-age=300";
/// The `max-age` of [`INDEX_CACHE`]: a cache may serve an index this long after it was replaced, so gc must
/// keep the objects that index references at least this long.
pub const INDEX_MAX_AGE_SECS: i64 = 300;
const GC_STATE_CACHE: &str = "no-store";

/// One channel root on a host. Keys are relative to it (`index.json`, `r/<sha>.json`, `s/<sha>.fds`).
pub trait Store {
    fn describe(&self) -> String;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    fn put(&self, key: &str, bytes: &[u8], content_type: &str, cache_control: &str) -> Result<()>;
    /// Keys under `prefix` (relative to the channel root).
    fn list(&self, prefix: &str) -> Result<Vec<String>>;
    fn delete(&self, key: &str) -> Result<()>;
}

pub struct DirStore {
    root: PathBuf,
}

impl DirStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Store for DirStore {
    fn describe(&self) -> String {
        self.root.display().to_string()
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.root.join(key)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {key}")),
        }
    }

    fn put(&self, key: &str, bytes: &[u8], _content_type: &str, _cache_control: &str) -> Result<()> {
        let path = self.root.join(key);
        std::fs::create_dir_all(path.parent().expect("store keys have a parent"))?;
        // Write-then-rename so a reader never sees a half-written index.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let dir = self.root.join(prefix);
        let mut out = Vec::new();
        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                for e in entries {
                    let e = e?;
                    if e.file_type()?.is_file() {
                        out.push(format!("{prefix}{}", e.file_name().to_string_lossy()));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("listing {}", dir.display())),
        }
        out.sort();
        Ok(out)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.root.join(key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("deleting {key}")),
        }
    }
}

/// An S3-compatible bucket, addressed path-style with presigned SigV4 URLs (`rusty-s3`) over `ureq`.
pub struct S3Store {
    agent: ureq::Agent,
    bucket: Bucket,
    creds: Credentials,
    /// Channel root inside the bucket, ending in `/`.
    prefix: String,
}

const SIGN_TTL: Duration = Duration::from_secs(600);

impl S3Store {
    pub fn new(endpoint: &str, region: &str, bucket: &str, prefix: &str, creds: Credentials) -> Result<Self> {
        let endpoint = url::Url::parse(endpoint).with_context(|| format!("bad endpoint `{endpoint}`"))?;
        let bucket = Bucket::new(endpoint, UrlStyle::Path, bucket.to_string(), region.to_string())
            .map_err(|e| anyhow!("bucket: {e}"))?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self {
            agent,
            bucket,
            creds,
            prefix: prefix.to_string(),
        })
    }

    fn full(&self, key: &str) -> String {
        format!("{}{key}", self.prefix)
    }
}

fn body_text(resp: &mut ureq::http::Response<ureq::Body>) -> String {
    let text = resp.body_mut().read_to_string().unwrap_or_default();
    text.chars().take(400).collect()
}

impl Store for S3Store {
    fn describe(&self) -> String {
        format!("s3://{}/{}", self.bucket.name(), self.prefix)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let full = self.full(key);
        let url = self.bucket.get_object(Some(&self.creds), &full).sign(SIGN_TTL);
        let mut resp = self
            .agent
            .get(url.as_str())
            .call()
            .with_context(|| format!("GET {full}"))?;
        match resp.status().as_u16() {
            200 => Ok(Some(
                resp.body_mut()
                    .with_config()
                    .limit(64 << 20)
                    .read_to_vec()
                    .with_context(|| format!("reading {full}"))?,
            )),
            404 => Ok(None),
            s => bail!("GET {full}: HTTP {s}: {}", body_text(&mut resp)),
        }
    }

    fn put(&self, key: &str, bytes: &[u8], content_type: &str, cache_control: &str) -> Result<()> {
        let full = self.full(key);
        let mut action = self.bucket.put_object(Some(&self.creds), &full);
        action.headers_mut().insert("content-type", content_type);
        action.headers_mut().insert("cache-control", cache_control);
        let url = action.sign(SIGN_TTL);
        let mut resp = self
            .agent
            .put(url.as_str())
            .header("content-type", content_type)
            .header("cache-control", cache_control)
            .send(bytes)
            .with_context(|| format!("PUT {full}"))?;
        match resp.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            s => bail!("PUT {full}: HTTP {s}: {}", body_text(&mut resp)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full_prefix = self.full(prefix);
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut action = self.bucket.list_objects_v2(Some(&self.creds));
            action.with_prefix(full_prefix.clone());
            if let Some(t) = token.take() {
                action.with_continuation_token(t);
            }
            let url = action.sign(SIGN_TTL);
            let mut resp = self
                .agent
                .get(url.as_str())
                .call()
                .with_context(|| format!("LIST {full_prefix}"))?;
            if resp.status().as_u16() != 200 {
                bail!("LIST {full_prefix}: HTTP {}: {}", resp.status(), body_text(&mut resp));
            }
            let xml = resp.body_mut().read_to_string()?;
            let parsed = rusty_s3::actions::ListObjectsV2::parse_response(&xml)
                .map_err(|e| anyhow!("LIST {full_prefix}: bad response: {e}"))?;
            for c in parsed.contents {
                if let Some(rel) = c.key.strip_prefix(&self.prefix) {
                    out.push(rel.to_string());
                }
            }
            match parsed.next_continuation_token {
                Some(t) => token = Some(t),
                None => break,
            }
        }
        out.sort();
        Ok(out)
    }

    fn delete(&self, key: &str) -> Result<()> {
        let full = self.full(key);
        let url = self.bucket.delete_object(Some(&self.creds), &full).sign(SIGN_TTL);
        let mut resp = self
            .agent
            .delete(url.as_str())
            .call()
            .with_context(|| format!("DELETE {full}"))?;
        match resp.status().as_u16() {
            200 | 204 | 404 => Ok(()),
            s => bail!("DELETE {full}: HTTP {s}: {}", body_text(&mut resp)),
        }
    }
}

/// Where a channel lives. `s3://bucket/prefix` or a local directory; both get `v1/<channel>/` appended.
pub fn open_store(target: &str, channel: &str, endpoint: Option<&str>, region: &str) -> Result<Box<dyn Store>> {
    if channel.is_empty()
        || !channel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("channel `{channel}` must be [A-Za-z0-9_-]+");
    }
    if let Some(rest) = target.strip_prefix("s3://") {
        let (bucket, base) = rest.split_once('/').unwrap_or((rest, ""));
        if bucket.is_empty() {
            bail!("`{target}`: missing bucket name");
        }
        let endpoint = endpoint
            .map(str::to_string)
            .or_else(|| std::env::var("REX_S3_ENDPOINT").ok())
            .ok_or_else(|| anyhow!("s3 targets need --endpoint or REX_S3_ENDPOINT"))?;
        let key = env_any(&["REX_S3_ACCESS_KEY_ID", "AWS_ACCESS_KEY_ID"])?;
        let secret = env_any(&["REX_S3_SECRET_ACCESS_KEY", "AWS_SECRET_ACCESS_KEY"])?;
        let base = base.trim_matches('/');
        let prefix = if base.is_empty() {
            format!("v1/{channel}/")
        } else {
            format!("{base}/v1/{channel}/")
        };
        Ok(Box::new(S3Store::new(
            &endpoint,
            region,
            bucket,
            &prefix,
            Credentials::new(key, secret),
        )?))
    } else if target.contains("://") {
        bail!("unsupported target `{target}` (use s3://bucket/prefix or a directory)");
    } else {
        Ok(Box::new(DirStore::new(PathBuf::from(target).join("v1").join(channel))))
    }
}

fn env_any(names: &[&str]) -> Result<String> {
    names
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
        .ok_or_else(|| anyhow!("set {}", names.join(" or ")))
}

/// `key_id:hex` (as accepted by `--trust`).
pub fn parse_trust(s: &str) -> Result<PinnedKey> {
    let (key_id, public) = s
        .split_once(':')
        .ok_or_else(|| anyhow!("--trust wants key_id:hex, got `{s}`"))?;
    let public =
        rex_bundle::parse_hex32(public).ok_or_else(|| anyhow!("--trust `{key_id}`: public key must be 64 hex"))?;
    Ok(PinnedKey {
        key_id: key_id.to_string(),
        public,
    })
}

fn content_type(key: &str) -> &'static str {
    if key.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

/// Paths an index makes clients fetch.
fn referenced(index: &pb::Index) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = index
        .entries
        .iter()
        .filter_map(|e| e.r#ref.as_ref())
        .map(|r| format!("r/{}.json", r.sha256))
        .collect();
    if let Some(s) = &index.schemas {
        out.insert(format!("s/{}.fds", s.sha256));
    }
    out
}

/// The parts of an index that matter to clients (everything except sequence and time).
fn same_content(a: &pb::Index, b: &pb::Index) -> bool {
    a.channel == b.channel
        && a.min_client_version == b.min_client_version
        && a.schemas == b.schemas
        && a.entries == b.entries
}

fn remote_index(store: &dyn Store, pool: &DescriptorPool, trusted: &[PinnedKey]) -> Result<Option<pb::Index>> {
    match store.get(INDEX)? {
        None => Ok(None),
        Some(bytes) => rex_bundle::verify_index(pool, &bytes, trusted).map(Some).map_err(|e| {
            anyhow!(
                "the published {}{INDEX} does not verify ({e}); refusing to touch it. \
                 Pass the old key with --trust key_id:hex during a key rotation.",
                store.describe()
            )
        }),
    }
}

pub struct PublishOptions<'a> {
    pub channel: &'a str,
    pub now: DateTime<Utc>,
    pub force: bool,
    pub dry_run: bool,
    pub min_client_version: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PublishReport {
    pub previous: Option<u64>,
    /// `None` when nothing changed and nothing was published.
    pub sequence: Option<u64>,
    pub uploaded: Vec<String>,
    pub skipped: usize,
    /// Objects that were listed but missing or corrupt on read-back and were uploaded again.
    pub repaired: Vec<String>,
}

/// Publish every recipe in `set` to `store`. `trusted` must include the signer's own key; extra keys let a
/// rotation accept an index signed by the previous key.
pub fn publish(
    store: &dyn Store,
    set: &RecipeSet,
    signer: &Signer25519,
    trusted: &[PinnedKey],
    opts: &PublishOptions,
) -> Result<PublishReport> {
    let pool = set.registry().pool();
    let previous = remote_index(store, pool, trusted)?;
    if let Some(p) = &previous {
        if p.channel != opts.channel {
            bail!(
                "{} holds channel `{}`, not `{}`",
                store.describe(),
                p.channel,
                opts.channel
            );
        }
    }
    let sequence = previous.as_ref().map_or(1, |p| p.sequence + 1);
    let build = BuildOptions {
        channel: opts.channel.to_string(),
        sequence,
        generated_at: prost_types::Timestamp {
            seconds: opts.now.timestamp(),
            nanos: 0,
        },
        min_client_version: opts.min_client_version.to_string(),
    };
    let files = rex_bundle::build(set, &build, signer)?;
    let (index_files, objects): (Vec<_>, Vec<_>) = files.into_iter().partition(|f| f.path == INDEX);
    let index_bytes = &index_files.first().expect("build emits index.json").bytes;
    let new_index = rex_bundle::verify_index(pool, index_bytes, trusted)
        .context("the freshly built index does not verify against the trusted keys (is --key pinned?)")?;
    if let Some(p) = &previous {
        if !opts.force && same_content(p, &new_index) {
            // Nothing to sign, but a channel left half-published (an object deleted, or truncated by an
            // interrupted upload) is healed here rather than staying broken until the recipes change.
            let repaired = ensure_objects(store, &objects, opts.dry_run)?;
            return Ok(PublishReport {
                previous: Some(p.sequence),
                sequence: None,
                uploaded: Vec::new(),
                skipped: objects.len() - repaired.len(),
                repaired,
            });
        }
    }

    let mut existing: BTreeSet<String> = store.list("r/")?.into_iter().collect();
    existing.extend(store.list("s/")?);
    // Objects that gc has marked as unreferenced may be deleted at any moment by a concurrent gc; upload them
    // again rather than relying on them (re-putting identical content-addressed bytes is harmless).
    let pending_gc = read_gc_state(store)?.unreferenced;
    let mut uploaded = Vec::new();
    let mut skipped = 0;
    for f in &objects {
        if existing.contains(&f.path) && !pending_gc.contains_key(&f.path) {
            skipped += 1;
            continue;
        }
        if !opts.dry_run {
            store.put(&f.path, &f.bytes, content_type(&f.path), OBJECT_CACHE)?;
        }
        uploaded.push(f.path.clone());
    }
    // Every object the new index references must read back intact before the index points clients at it.
    let mut repaired = ensure_objects(store, &objects, opts.dry_run)?;
    repaired.retain(|p| !uploaded.contains(p)); // a dry run has not put the new ones
    skipped -= repaired.len();
    if !opts.dry_run {
        store.put(INDEX, index_bytes, "application/json", INDEX_CACHE)?;
        let back = store
            .get(INDEX)?
            .ok_or_else(|| anyhow!("{INDEX} vanished right after upload"))?;
        let check = rex_bundle::verify_index(pool, &back, trusted).context("read-back of the published index")?;
        if check.sequence != sequence {
            bail!(
                "read-back sequence {} != published {sequence} (concurrent publish?)",
                check.sequence
            );
        }
    }
    uploaded.push(INDEX.to_string());
    Ok(PublishReport {
        previous: previous.map(|p| p.sequence),
        sequence: Some(sequence),
        uploaded,
        skipped,
        repaired,
    })
}

/// Reads every object back and uploads again any that is missing or does not match its hash (once; a second
/// mismatch is an error). Returns the paths it uploaded. In a dry run, reports them without uploading.
fn ensure_objects(store: &dyn Store, objects: &[rex_bundle::BuiltFile], dry_run: bool) -> Result<Vec<String>> {
    let intact = |path: &str| -> Result<bool> {
        Ok(store
            .get(path)?
            .is_some_and(|b| rex_bundle::sha256_hex(&b) == sha_of(path)))
    };
    let mut repaired = Vec::new();
    for f in objects {
        if intact(&f.path)? {
            continue;
        }
        repaired.push(f.path.clone());
        if dry_run {
            continue;
        }
        store.put(&f.path, &f.bytes, content_type(&f.path), OBJECT_CACHE)?;
        if !intact(&f.path)? {
            bail!("read-back of {} failed or does not match its hash", f.path);
        }
    }
    Ok(repaired)
}

/// `r/<sha>.json` -> `<sha>`.
fn sha_of(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
pub struct GcState {
    #[serde(default)]
    pub format: String,
    /// Object path -> when gc first saw it unreferenced (RFC 3339).
    #[serde(default)]
    pub unreferenced: BTreeMap<String, String>,
}

const GC_FORMAT: &str = "rex.bundle.gc.v1";

fn read_gc_state(store: &dyn Store) -> Result<GcState> {
    match store.get(GC_STATE)? {
        None => Ok(GcState::default()),
        Some(b) => serde_json::from_slice(&b).with_context(|| format!("parsing {GC_STATE}")),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub sequence: u64,
    pub referenced: usize,
    pub newly_unreferenced: Vec<String>,
    pub waiting: Vec<String>,
    pub deleted: Vec<String>,
}

/// Mark objects the verified index no longer references; delete those unreferenced for at least `retention`.
pub fn gc(
    store: &dyn Store,
    pool: &DescriptorPool,
    trusted: &[PinnedKey],
    now: DateTime<Utc>,
    retention: chrono::Duration,
    dry_run: bool,
) -> Result<GcReport> {
    if retention < chrono::Duration::seconds(INDEX_MAX_AGE_SECS) {
        bail!(
            "retention {}s is shorter than the index cache lifetime ({INDEX_MAX_AGE_SECS}s): a cached index could \
             still point clients at deleted objects",
            retention.num_seconds()
        );
    }
    let index = remote_index(store, pool, trusted)?
        .ok_or_else(|| anyhow!("no {INDEX} at {}; nothing to collect against", store.describe()))?;
    let live = referenced(&index);
    let mut objects = store.list("r/")?;
    objects.extend(store.list("s/")?);
    let mut state = read_gc_state(store)?;
    let mut report = GcReport {
        sequence: index.sequence,
        referenced: live.len(),
        ..GcReport::default()
    };
    let mut next = BTreeMap::new();
    for path in objects {
        if live.contains(&path) {
            continue;
        }
        let since = match state.unreferenced.remove(&path) {
            Some(s) => DateTime::parse_from_rfc3339(&s)
                .with_context(|| format!("{GC_STATE}: bad time for {path}"))?
                .with_timezone(&Utc),
            None => {
                report.newly_unreferenced.push(path.clone());
                now
            }
        };
        if now - since >= retention {
            report.deleted.push(path);
        } else {
            report.waiting.push(path.clone());
            next.insert(path, since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        }
    }
    if !dry_run {
        // Re-check right before deleting: a publish since our read may reference these again.
        let fresh = remote_index(store, pool, trusted)?
            .map(|i| referenced(&i))
            .unwrap_or_default();
        report.deleted.retain(|p| !fresh.contains(p));
        for path in &report.deleted {
            store.delete(path)?;
        }
        state.format = GC_FORMAT.to_string();
        state.unreferenced = next;
        store.put(
            GC_STATE,
            serde_json::to_string_pretty(&state)?.as_bytes(),
            "application/json",
            GC_STATE_CACHE,
        )?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_schema::SchemaRegistry;

    const BASE: &str = "id: generic/article.v1\nversion: 1\nschema: rex.schema.article.v1.Article\nfields:\n  - path: title\n    candidates: [ { json_ld: {} } ]\n";

    fn site(version: u32, selector: &str) -> String {
        format!(
            "id: example.com/article\nversion: {version}\nschema: rex.schema.article.v1.Article\nextends: generic/article.v1\nmatch:\n  urls: {{ hosts: [example.com] }}\nfields:\n  - path: title\n    candidates: [ {{ css: {{ selector: \"{selector}\" }} }} ]\n"
        )
    }

    fn recipes(site_yaml: &str) -> RecipeSet {
        let mut s = RecipeSet::new(SchemaRegistry::embedded().unwrap());
        s.add_yaml(BASE).unwrap();
        s.add_yaml(site_yaml).unwrap();
        s
    }

    fn signer() -> (Signer25519, Vec<PinnedKey>) {
        let s = Signer25519::from_seed("test-1", [3u8; 32]);
        let pinned = vec![PinnedKey {
            key_id: "test-1".into(),
            public: s.public_key(),
        }];
        (s, pinned)
    }

    fn store() -> (tempfile::TempDir, DirStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = DirStore::new(dir.path().join("v1/dev"));
        (dir, store)
    }

    fn at(day: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-10-{day:02}T00:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn opts(now: DateTime<Utc>) -> PublishOptions<'static> {
        PublishOptions {
            channel: "dev",
            now,
            force: false,
            dry_run: false,
            min_client_version: "0.1.0",
        }
    }

    #[test]
    fn first_publish_writes_sequence_one_that_verifies() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let set = recipes(&site(1, "h1"));

        // Act
        let report = publish(&store, &set, &signer, &pinned, &opts(at(1))).unwrap();

        // Assert
        assert_eq!(report.previous, None);
        assert_eq!(report.sequence, Some(1));
        assert_eq!(report.uploaded.len(), 4, "two recipes, one fds, the index");
        let index = rex_bundle::verify_index(set.registry().pool(), &store.get(INDEX).unwrap().unwrap(), &pinned);
        assert_eq!(index.unwrap().sequence, 1);
    }

    #[test]
    fn republishing_unchanged_recipes_is_a_no_op() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let set = recipes(&site(1, "h1"));
        publish(&store, &set, &signer, &pinned, &opts(at(1))).unwrap();

        // Act
        let report = publish(&store, &set, &signer, &pinned, &opts(at(2))).unwrap();

        // Assert
        assert_eq!(report.sequence, None);
        assert!(report.uploaded.is_empty());
    }

    #[test]
    fn changed_recipe_bumps_sequence_and_uploads_only_new_objects() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        publish(&store, &recipes(&site(1, "h1")), &signer, &pinned, &opts(at(1))).unwrap();

        // Act
        let report = publish(&store, &recipes(&site(2, "h1.title")), &signer, &pinned, &opts(at(2))).unwrap();

        // Assert
        assert_eq!(report.previous, Some(1));
        assert_eq!(report.sequence, Some(2));
        assert_eq!(report.skipped, 2, "base recipe and fds are already there");
        assert_eq!(report.uploaded.len(), 2, "the new site recipe and the index");
    }

    #[test]
    fn publish_refuses_an_index_signed_by_an_unknown_key() {
        // Arrange
        let (_dir, store) = store();
        let stranger = Signer25519::from_seed("other", [9u8; 32]);
        let stranger_pin = vec![PinnedKey {
            key_id: "other".into(),
            public: stranger.public_key(),
        }];
        publish(&store, &recipes(&site(1, "h1")), &stranger, &stranger_pin, &opts(at(1))).unwrap();
        let (signer, pinned) = signer();

        // Act
        let r = publish(&store, &recipes(&site(2, "h2")), &signer, &pinned, &opts(at(2)));

        // Assert
        let err = r.unwrap_err().to_string();
        assert!(err.contains("does not verify"), "{err}");
    }

    #[test]
    fn rotation_accepts_the_old_key_via_trust_and_continues_the_sequence() {
        // Arrange
        let (_dir, store) = store();
        let old = Signer25519::from_seed("old", [5u8; 32]);
        let old_pin = PinnedKey {
            key_id: "old".into(),
            public: old.public_key(),
        };
        publish(
            &store,
            &recipes(&site(1, "h1")),
            &old,
            std::slice::from_ref(&old_pin),
            &opts(at(1)),
        )
        .unwrap();
        let (signer, mut pinned) = signer();
        pinned.push(old_pin);

        // Act
        let report = publish(&store, &recipes(&site(2, "h2")), &signer, &pinned, &opts(at(2))).unwrap();

        // Assert
        assert_eq!(report.sequence, Some(2));
    }

    #[test]
    fn unchanged_publish_repairs_a_missing_or_truncated_object() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let set = recipes(&site(1, "h1"));
        publish(&store, &set, &signer, &pinned, &opts(at(1))).unwrap();
        let objects = store.list("r/").unwrap();
        store.delete(&objects[0]).unwrap();
        store.put(&objects[1], b"{", "application/json", OBJECT_CACHE).unwrap();

        // Act
        let report = publish(&store, &set, &signer, &pinned, &opts(at(2))).unwrap();

        // Assert
        assert_eq!(report.sequence, None, "nothing to re-sign");
        assert_eq!(report.repaired, objects[..2].to_vec());
        for path in &objects {
            let bytes = store.get(path).unwrap().unwrap();
            assert_eq!(rex_bundle::sha256_hex(&bytes), sha_of(path));
        }
    }

    #[test]
    fn publish_repairs_a_corrupt_listed_object_before_writing_the_index() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        publish(&store, &recipes(&site(1, "h1")), &signer, &pinned, &opts(at(1))).unwrap();
        let fds = store.list("s/").unwrap().remove(0);
        store
            .put(&fds, b"truncated", "application/octet-stream", OBJECT_CACHE)
            .unwrap();

        // Act
        let report = publish(&store, &recipes(&site(2, "h2")), &signer, &pinned, &opts(at(2))).unwrap();

        // Assert
        assert_eq!(report.sequence, Some(2));
        assert_eq!(report.repaired, vec![fds.clone()]);
        assert_eq!(rex_bundle::sha256_hex(&store.get(&fds).unwrap().unwrap()), sha_of(&fds));
    }

    #[test]
    fn gc_refuses_a_retention_shorter_than_the_index_cache_lifetime() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let pool = SchemaRegistry::embedded().unwrap().pool().clone();
        publish(&store, &recipes(&site(1, "h1")), &signer, &pinned, &opts(at(1))).unwrap();

        // Act
        let r = gc(&store, &pool, &pinned, at(2), chrono::Duration::seconds(10), false);

        // Assert
        let err = r.unwrap_err().to_string();
        assert!(err.contains("index cache lifetime"), "{err}");
        assert!(INDEX_CACHE.contains(&format!("max-age={INDEX_MAX_AGE_SECS}")));
    }

    #[test]
    fn gc_keeps_unreferenced_objects_until_retention_then_deletes_them() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let pool = SchemaRegistry::embedded().unwrap().pool().clone();
        let v1 = recipes(&site(1, "h1"));
        publish(&store, &v1, &signer, &pinned, &opts(at(1))).unwrap();
        publish(&store, &recipes(&site(2, "h2")), &signer, &pinned, &opts(at(2))).unwrap();
        let first = gc(&store, &pool, &pinned, at(3), chrono::Duration::days(7), false).unwrap();

        // Act
        let second = gc(&store, &pool, &pinned, at(11), chrono::Duration::days(7), false).unwrap();

        // Assert
        assert_eq!(first.newly_unreferenced.len(), 1, "the v1 site recipe");
        assert!(first.deleted.is_empty());
        assert_eq!(second.deleted, first.newly_unreferenced);
        assert!(store.get(&second.deleted[0]).unwrap().is_none());
        assert_eq!(store.list("r/").unwrap().len(), 2, "base + v2 site recipe survive");
    }

    #[test]
    fn publish_reuploads_an_object_that_gc_has_marked() {
        // Arrange
        let (_dir, store) = store();
        let (signer, pinned) = signer();
        let pool = SchemaRegistry::embedded().unwrap().pool().clone();
        publish(&store, &recipes(&site(1, "h1")), &signer, &pinned, &opts(at(1))).unwrap();
        publish(&store, &recipes(&site(2, "h2")), &signer, &pinned, &opts(at(2))).unwrap();
        let marked = gc(&store, &pool, &pinned, at(3), chrono::Duration::days(7), false).unwrap();

        // Act: reverting to the v1 content references the object gc just marked.
        let report = publish(&store, &recipes(&site(1, "h1")), &signer, &pinned, &opts(at(4))).unwrap();

        // Assert
        assert!(report.uploaded.contains(&marked.newly_unreferenced[0]), "{report:?}");
    }
}
