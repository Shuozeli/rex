//! Recipe bundles (decisions.md U16, `proto/rex/bundle/v1/bundle.proto`).
//!
//! Layout under `<base>/v1/<channel>/`:
//!   index.json        SignedIndex (protojson); Ed25519 over `payload` = canonical protojson of Index
//!   r/<sha256>.json   one ExtractionRecipe, canonical protojson (the hashed bytes are the file bytes)
//!   s/<sha256>.fds    FileDescriptorSet with every schema the index references
//!
//! The extension ships its recipes inside the package instead (decisions.md U45): [`embedded`] writes the same
//! Index, unsigned (Chrome verifies the package), with the schema set and the recipes in index order.
//!
//! The library has no clock: `generated_at` and `sequence` are supplied by the caller.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use prost_reflect::{DescriptorPool, DynamicMessage, SerializeOptions};
use rex_engine::recipe::{canonical_json, is_base_recipe};
use rex_engine::RecipeSet;
use rex_proto::rex::bundle::v1 as pb;
use rex_proto::rex::extract::v1 as ex;
use sha2::{Digest, Sha256};

pub const INDEX_FORMAT: &str = "rex.bundle.index.v1";
pub const ALG: &str = "Ed25519";

#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("recipe `{0}`: {1}")]
    Recipe(String, String),
    #[error("index: {0}")]
    Index(String),
    #[error("signature: {0}")]
    Signature(String),
}

/// A signing key with its id (the id is what clients pin).
pub struct Signer25519 {
    pub key_id: String,
    key: SigningKey,
}

impl Signer25519 {
    pub fn from_seed(key_id: &str, seed: [u8; 32]) -> Self {
        Self {
            key_id: key_id.to_string(),
            key: SigningKey::from_bytes(&seed),
        }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
}

/// A public key a client trusts.
#[derive(Clone, Debug)]
pub struct PinnedKey {
    pub key_id: String,
    pub public: [u8; 32],
}

pub struct BuildOptions {
    pub channel: String,
    pub sequence: u64,
    pub generated_at: prost_types::Timestamp,
    pub min_client_version: String,
}

#[derive(Debug, Clone)]
pub struct BuiltFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn pool_of(set: &RecipeSet) -> &DescriptorPool {
    set.registry().pool()
}

fn to_protojson<M: prost::Message>(pool: &DescriptorPool, name: &str, msg: &M) -> Result<String, String> {
    let desc = pool
        .get_message_by_name(name)
        .ok_or_else(|| format!("descriptor pool lacks {name}"))?;
    let mut dynamic = DynamicMessage::new(desc);
    dynamic.transcode_from(msg).map_err(|e| e.to_string())?;
    let value = dynamic
        .serialize_with_options(serde_json::value::Serializer, &SerializeOptions::new())
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn from_protojson<M: prost::Message + Default>(pool: &DescriptorPool, name: &str, json: &[u8]) -> Result<M, String> {
    let desc = pool
        .get_message_by_name(name)
        .ok_or_else(|| format!("descriptor pool lacks {name}"))?;
    let mut de = serde_json::Deserializer::from_slice(json);
    let dynamic = DynamicMessage::deserialize(desc, &mut de).map_err(|e| e.to_string())?;
    dynamic.transcode_to::<M>().map_err(|e| e.to_string())
}

/// The Index for `set` and its objects: `r/<sha>.json` per recipe (in index order), then `s/<sha>.fds`.
fn index_and_objects(set: &RecipeSet, opts: &BuildOptions) -> Result<(pb::Index, Vec<BuiltFile>), BundleError> {
    let mut files = Vec::new();
    let mut entries = Vec::new();
    for id in set.ids() {
        let recipe = set.get(id).expect("listed id exists");
        let (json, sha) = canonical_json(recipe, set.registry()).map_err(|e| BundleError::Recipe(id.to_string(), e))?;
        let bytes = json.into_bytes();
        debug_assert_eq!(sha, sha256_hex(&bytes));
        entries.push(pb::IndexEntry {
            r#ref: Some(ex::RecipeRef {
                id: recipe.id.clone(),
                version: recipe.version,
                sha256: sha.clone(),
            }),
            schema: recipe.schema.clone(),
            extends: recipe.extends.clone(),
            urls: recipe.r#match.as_ref().and_then(|m| m.urls.clone()),
            priority: recipe.r#match.as_ref().map(|m| m.priority).unwrap_or(0),
            requires: recipe.requires.clone(),
            bytes: bytes.len() as u64,
            base: is_base_recipe(recipe),
        });
        files.push(BuiltFile {
            path: format!("r/{sha}.json"),
            bytes,
        });
    }
    let fds = rex_proto::FILE_DESCRIPTOR_SET.to_vec();
    let fds_sha = sha256_hex(&fds);
    let index = pb::Index {
        format: INDEX_FORMAT.to_string(),
        channel: opts.channel.clone(),
        sequence: opts.sequence,
        generated_at: Some(opts.generated_at),
        min_client_version: opts.min_client_version.clone(),
        schemas: Some(pb::SchemaSet {
            sha256: fds_sha.clone(),
            bytes: fds.len() as u64,
            messages: set.registry().schemas(),
        }),
        entries,
    };
    files.push(BuiltFile {
        path: format!("s/{fds_sha}.fds"),
        bytes: fds,
    });
    Ok((index, files))
}

/// Build every file of a bundle for one channel. Every recipe in `set` is published.
pub fn build(set: &RecipeSet, opts: &BuildOptions, signer: &Signer25519) -> Result<Vec<BuiltFile>, BundleError> {
    let pool = pool_of(set);
    let (index, mut files) = index_and_objects(set, opts)?;
    let payload = to_protojson(pool, "rex.bundle.v1.Index", &index)
        .map_err(BundleError::Index)?
        .into_bytes();
    let signature = signer.key.sign(&payload).to_bytes().to_vec();
    let signed = pb::SignedIndex {
        key_id: signer.key_id.clone(),
        alg: ALG.to_string(),
        signature,
        payload,
    };
    let index_json = to_protojson(pool, "rex.bundle.v1.SignedIndex", &signed).map_err(BundleError::Index)?;
    files.push(BuiltFile {
        path: "index.json".to_string(),
        bytes: index_json.into_bytes(),
    });
    Ok(files)
}

/// The recipes as the extension ships them (decisions.md U45): `index.json` (the Index as protojson, unsigned),
/// `schemas.fds`, and `recipes.json` (the canonical recipe protojson strings, in index order).
pub fn embedded(set: &RecipeSet, opts: &BuildOptions) -> Result<Vec<BuiltFile>, BundleError> {
    let pool = pool_of(set);
    let (index, objects) = index_and_objects(set, opts)?;
    let mut recipes = Vec::new();
    let mut fds = Vec::new();
    for f in objects {
        if f.path.starts_with("r/") {
            recipes.push(String::from_utf8(f.bytes).map_err(|e| BundleError::Index(e.to_string()))?);
        } else {
            fds = f.bytes;
        }
    }
    let recipes_json = serde_json::to_string(&recipes).map_err(|e| BundleError::Index(e.to_string()))?;
    Ok(vec![
        BuiltFile {
            path: "index.json".into(),
            bytes: index_to_json(pool, &index)?.into_bytes(),
        },
        BuiltFile {
            path: "schemas.fds".into(),
            bytes: fds,
        },
        BuiltFile {
            path: "recipes.json".into(),
            bytes: recipes_json.into_bytes(),
        },
    ])
}

/// Verify `index.json` against pinned keys and return the Index.
pub fn verify_index(pool: &DescriptorPool, index_json: &[u8], keys: &[PinnedKey]) -> Result<pb::Index, BundleError> {
    let signed: pb::SignedIndex =
        from_protojson(pool, "rex.bundle.v1.SignedIndex", index_json).map_err(BundleError::Index)?;
    if signed.alg != ALG {
        return Err(BundleError::Signature(format!("unsupported alg `{}`", signed.alg)));
    }
    let key = keys
        .iter()
        .find(|k| k.key_id == signed.key_id)
        .ok_or_else(|| BundleError::Signature(format!("key `{}` is not pinned", signed.key_id)))?;
    let vk = VerifyingKey::from_bytes(&key.public).map_err(|e| BundleError::Signature(e.to_string()))?;
    let sig_bytes: [u8; 64] = signed
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| BundleError::Signature("signature is not 64 bytes".into()))?;
    vk.verify(&signed.payload, &Signature::from_bytes(&sig_bytes))
        .map_err(|_| BundleError::Signature("signature does not verify against the pinned key".into()))?;
    let index: pb::Index = from_protojson(pool, "rex.bundle.v1.Index", &signed.payload).map_err(BundleError::Index)?;
    if index.format != INDEX_FORMAT {
        return Err(BundleError::Index(format!("unknown format `{}`", index.format)));
    }
    Ok(index)
}

/// Protojson of an Index (what clients store after verification).
pub fn index_to_json(pool: &DescriptorPool, index: &pb::Index) -> Result<String, BundleError> {
    to_protojson(pool, "rex.bundle.v1.Index", index).map_err(BundleError::Index)
}

pub fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_schema::SchemaRegistry;

    fn set() -> RecipeSet {
        let mut s = RecipeSet::new(SchemaRegistry::embedded().unwrap());
        s.add_yaml(
            "id: generic/article.v1\nversion: 1\nschema: rex.schema.article.v1.Article\nfields:\n  - path: title\n    candidates: [ { json_ld: {} } ]\n",
        )
        .unwrap();
        s
    }

    fn opts(sequence: u64) -> BuildOptions {
        BuildOptions {
            channel: "dev".into(),
            sequence,
            generated_at: prost_types::Timestamp {
                seconds: 1_791_700_000,
                nanos: 0,
            },
            min_client_version: "0.1.0".into(),
        }
    }

    #[test]
    fn embedded_bundle_lists_each_recipe_once_in_index_order() {
        // Arrange
        let s = set();

        // Act
        let files = embedded(&s, &opts(1)).unwrap();

        // Assert
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["index.json", "schemas.fds", "recipes.json"]);
        let index: serde_json::Value = serde_json::from_slice(&files[0].bytes).unwrap();
        let recipes: Vec<String> = serde_json::from_slice(&files[2].bytes).unwrap();
        assert_eq!(index["entries"].as_array().unwrap().len(), recipes.len());
        assert_eq!(index["entries"][0]["ref"]["id"], "generic/article.v1");
        assert!(recipes[0].contains("\"generic/article.v1\""));
        assert_eq!(index["schemas"]["sha256"], sha256_hex(&files[1].bytes));
    }

    #[test]
    fn built_bundle_verifies_and_lists_entries() {
        // Arrange
        let s = set();
        let signer = Signer25519::from_seed("dev-1", [7u8; 32]);
        let pinned = vec![PinnedKey {
            key_id: "dev-1".into(),
            public: signer.public_key(),
        }];

        // Act
        let files = build(&s, &opts(3), &signer).unwrap();
        let index_json = &files.iter().find(|f| f.path == "index.json").unwrap().bytes;
        let index = verify_index(s.registry().pool(), index_json, &pinned).unwrap();

        // Assert
        assert_eq!(index.sequence, 3);
        assert_eq!(index.entries.len(), 1);
        let entry = &index.entries[0];
        assert!(entry.base);
        let sha = &entry.r#ref.as_ref().unwrap().sha256;
        let recipe_file = files.iter().find(|f| f.path == format!("r/{sha}.json")).unwrap();
        assert_eq!(&sha256_hex(&recipe_file.bytes), sha);
    }

    #[test]
    fn tampered_payload_is_rejected() {
        // Arrange
        let s = set();
        let signer = Signer25519::from_seed("dev-1", [7u8; 32]);
        let pinned = vec![PinnedKey {
            key_id: "dev-1".into(),
            public: signer.public_key(),
        }];
        let files = build(&s, &opts(1), &signer).unwrap();
        let index_json =
            String::from_utf8(files.iter().find(|f| f.path == "index.json").unwrap().bytes.clone()).unwrap();
        let mut signed: serde_json::Value = serde_json::from_str(&index_json).unwrap();
        signed["keyId"] = serde_json::Value::String("dev-1".into());
        signed["signature"] = serde_json::Value::String(hex_to_b64_flip(signed["signature"].as_str().unwrap()));

        // Act
        let r = verify_index(s.registry().pool(), signed.to_string().as_bytes(), &pinned);

        // Assert
        assert!(matches!(r, Err(BundleError::Signature(_))));
    }

    #[test]
    fn unpinned_key_is_rejected() {
        // Arrange
        let s = set();
        let signer = Signer25519::from_seed("dev-1", [7u8; 32]);
        let other = Signer25519::from_seed("dev-2", [9u8; 32]);
        let pinned = vec![PinnedKey {
            key_id: "dev-2".into(),
            public: other.public_key(),
        }];
        let files = build(&s, &opts(1), &signer).unwrap();

        // Act
        let r = verify_index(
            s.registry().pool(),
            &files.iter().find(|f| f.path == "index.json").unwrap().bytes,
            &pinned,
        );

        // Assert
        assert!(matches!(r, Err(BundleError::Signature(_))));
    }

    /// Flip the first base64 character of a signature (protojson bytes are base64).
    fn hex_to_b64_flip(b64: &str) -> String {
        let mut chars: Vec<char> = b64.chars().collect();
        chars[0] = if chars[0] == 'A' { 'B' } else { 'A' };
        chars.into_iter().collect()
    }
}
