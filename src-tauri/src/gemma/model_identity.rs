use super::{
    gguf_selection, is_gguf_file, is_model_asset_file, local_model_label, model_name_from_config,
    GemmaError, MODEL_CONFIG, TOKENIZER, TOKENIZER_CONFIG,
};
use llama_cpp_2::gguf::GgufContext;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

const IDENTITY_PREFIX_BYTES: u64 = 256 * 1024;
const IDENTITY_SUFFIX_BYTES: u64 = 64 * 1024;
const IDENTITY_DIGEST_HEX_BYTES: usize = 12;

pub const GEMMA_E2B_CANONICAL_ID: &str = "gemma-4-E2B-it-qat-q4_0-gguf";
pub const GEMMA_E2B_FULL_DISPLAY_NAME: &str = "Gemma 4 E2B IT QAT Q4_0 GGUF";
pub const GEMMA_E4B_CANONICAL_ID: &str = "gemma-4-E4B-it-qat-q4_0-gguf";
pub const GEMMA_12B_CANONICAL_ID: &str = "gemma-4-12B-it-qat-q4_0-gguf";
pub const CLEAN_INSTALL_STARTUP_MODEL_ID: &str = GEMMA_E2B_CANONICAL_ID;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalModelIdentitySource {
    CanonicalRegistry,
    ModelMetadata,
    WeightMetadata,
    StorageMetadata,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelIdentity {
    pub canonical_id: String,
    pub display_name: String,
    pub storage_directory: PathBuf,
    pub source: LocalModelIdentitySource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyIdentityResolution {
    Unique(LocalModelIdentity),
    Ambiguous,
    Unavailable,
}

impl LocalModelIdentity {
    pub fn canonical_id(&self) -> &str {
        &self.canonical_id
    }
}

pub fn identity_for_model_directory(model_dir: &Path) -> Result<LocalModelIdentity, GemmaError> {
    if !model_dir.is_dir() {
        return Err(GemmaError {
            code: "local_model_not_found",
            message: "The requested model was not found in the configured local-model store."
                .to_string(),
        });
    }

    let configured_name = model_name_from_config(model_dir);
    let selected_weight = gguf_selection::select_primary_gguf(model_dir)?;
    let gguf_metadata = selected_weight
        .as_deref()
        .and_then(|path| gguf_identity_metadata(path).ok());
    let weight_stem = selected_weight
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty());
    for candidate in [
        gguf_metadata
            .as_ref()
            .map(|metadata| metadata.name.as_str()),
        configured_name.as_deref(),
        weight_stem,
    ]
    .into_iter()
    .flatten()
    {
        if let Some(canonical_id) = canonical_registry_match(candidate) {
            return Ok(LocalModelIdentity {
                canonical_id: canonical_id.to_string(),
                display_name: canonical_display_name(canonical_id),
                storage_directory: model_dir.to_path_buf(),
                source: LocalModelIdentitySource::CanonicalRegistry,
            });
        }
    }

    if let Some(metadata) = gguf_metadata {
        return Ok(metadata_identity(model_dir, metadata));
    }

    if let Some(configured_name) = configured_name {
        return Ok(LocalModelIdentity {
            canonical_id: stable_custom_id(&configured_name),
            display_name: configured_name,
            storage_directory: model_dir.to_path_buf(),
            source: LocalModelIdentitySource::ModelMetadata,
        });
    }
    if let Some(weight_stem) = weight_stem {
        let canonical_id = stable_custom_id(weight_stem);
        return Ok(LocalModelIdentity {
            display_name: local_model_label(&canonical_id),
            canonical_id,
            storage_directory: model_dir.to_path_buf(),
            source: LocalModelIdentitySource::WeightMetadata,
        });
    }
    Err(GemmaError {
        code: "local_model_identity_ambiguous",
        message: "OOMU could not verify a stable identity for this local model.".to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GgufIdentityMetadata {
    name: String,
    architecture: String,
    digest: String,
}

fn gguf_identity_metadata(weight_path: &Path) -> Result<GgufIdentityMetadata, GemmaError> {
    let gguf = GgufContext::from_file(weight_path).ok_or_else(|| GemmaError {
        code: "local_model_gguf_metadata_unavailable",
        message: "OOMU could not read this model's GGUF metadata.".to_string(),
    })?;
    let architecture =
        gguf_string(&gguf, "general.architecture").unwrap_or_else(|| "model".to_string());
    let name = gguf_string(&gguf, "general.name")
        .or_else(|| gguf_string(&gguf, "general.basename"))
        .unwrap_or_else(|| "Local GGUF".to_string());
    let digest = bounded_gguf_identity_digest(weight_path, &gguf, &architecture, &name)?;
    Ok(GgufIdentityMetadata {
        name,
        architecture,
        digest,
    })
}

fn gguf_string(gguf: &GgufContext, key: &str) -> Option<String> {
    let index = gguf.find_key(key);
    (index >= 0)
        .then(|| gguf.val_str(index))
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn bounded_gguf_identity_digest(
    weight_path: &Path,
    gguf: &GgufContext,
    architecture: &str,
    name: &str,
) -> Result<String, GemmaError> {
    let metadata = fs::metadata(weight_path)
        .map_err(|error| GemmaError::io("local model identity metadata", error))?;
    let mut file = File::open(weight_path)
        .map_err(|error| GemmaError::io("local model identity open", error))?;
    let mut hasher = Sha256::new();
    for value in [
        "oomu.gguf.identity.v1".to_string(),
        architecture.to_string(),
        name.to_string(),
        gguf.n_tensors().to_string(),
        metadata.len().to_string(),
    ] {
        hasher.update(value.len().to_le_bytes());
        hasher.update(value.as_bytes());
    }

    hash_file_window(&mut file, &mut hasher, 0, IDENTITY_PREFIX_BYTES)?;
    if metadata.len() > IDENTITY_PREFIX_BYTES {
        hash_file_window(
            &mut file,
            &mut hasher,
            metadata.len().saturating_sub(IDENTITY_SUFFIX_BYTES),
            IDENTITY_SUFFIX_BYTES,
        )?;
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_file_window(
    file: &mut File,
    hasher: &mut Sha256,
    offset: u64,
    maximum_bytes: u64,
) -> Result<(), GemmaError> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| GemmaError::io("local model identity seek", error))?;
    let mut remaining = maximum_bytes;
    let mut buffer = [0_u8; 16 * 1024];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file
            .read(&mut buffer[..wanted])
            .map_err(|error| GemmaError::io("local model identity read", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        remaining = remaining.saturating_sub(read as u64);
    }
    Ok(())
}

fn metadata_identity(model_dir: &Path, metadata: GgufIdentityMetadata) -> LocalModelIdentity {
    let architecture = stable_custom_id(&metadata.architecture.to_ascii_lowercase());
    let digest_length = IDENTITY_DIGEST_HEX_BYTES * 2;
    let digest = metadata
        .digest
        .get(..digest_length)
        .unwrap_or(metadata.digest.as_str());
    LocalModelIdentity {
        canonical_id: format!(
            "gguf-{}-{digest}",
            if architecture.is_empty() {
                "model"
            } else {
                architecture.as_str()
            }
        ),
        display_name: metadata.name,
        storage_directory: model_dir.to_path_buf(),
        source: LocalModelIdentitySource::WeightMetadata,
    }
}

pub(super) fn model_directory_matches_legacy_reference(model_dir: &Path, reference: &str) -> bool {
    let reference = reference.trim();
    if reference.is_empty() {
        return false;
    }
    let configured_name = model_name_from_config(model_dir);
    let weight_stem = gguf_selection::select_primary_gguf(model_dir)
        .ok()
        .flatten()
        .as_deref()
        .and_then(Path::file_stem)
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned);
    [configured_name, weight_stem]
        .into_iter()
        .flatten()
        .map(|candidate| stable_custom_id(&candidate))
        .any(|legacy_id| legacy_id.eq_ignore_ascii_case(reference))
}

pub(super) fn directory_has_model_evidence(model_dir: &Path) -> bool {
    fs::read_dir(model_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .any(|path| {
            path.is_file()
                && (is_model_asset_file(&path)
                    || path.file_name().is_some_and(|name| {
                        matches!(
                            name.to_str(),
                            Some(MODEL_CONFIG | TOKENIZER | TOKENIZER_CONFIG)
                        )
                    }))
        })
}

pub fn resolve_legacy_identity(
    model_root: &Path,
    reference: &str,
) -> Result<LegacyIdentityResolution, GemmaError> {
    let reference = reference.trim();
    if reference.is_empty() || reference.eq_ignore_ascii_case("dynamic") {
        return Ok(LegacyIdentityResolution::Unavailable);
    }
    let referenced_path = Path::new(reference);
    if referenced_path.is_absolute() {
        return resolve_absolute_identity(model_root, referenced_path);
    }
    if is_opaque_storage_reference(reference) {
        return resolve_opaque_identity(model_root);
    }

    resolve_named_identity(model_root, reference)
}

fn resolve_absolute_identity(
    model_root: &Path,
    referenced_path: &Path,
) -> Result<LegacyIdentityResolution, GemmaError> {
    let canonical_root = model_root
        .canonicalize()
        .map_err(|error| GemmaError::io("local model root canonicalization", error))?;
    let canonical_path = match referenced_path.canonicalize() {
        Ok(path) => path,
        Err(_) => return Ok(LegacyIdentityResolution::Unavailable),
    };
    if canonical_path != canonical_root && !canonical_path.starts_with(&canonical_root) {
        return Ok(LegacyIdentityResolution::Unavailable);
    }
    if canonical_path.is_dir() {
        return identity_for_model_directory(&canonical_path).map(LegacyIdentityResolution::Unique);
    }
    if !canonical_path.is_file() || !is_gguf_file(&canonical_path) {
        return Ok(LegacyIdentityResolution::Unavailable);
    }
    let filename = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ["mmproj", "draft", "mtp"]
        .iter()
        .any(|part| filename.contains(part))
    {
        return Ok(LegacyIdentityResolution::Unavailable);
    }
    let Some(parent) = canonical_path.parent() else {
        return Ok(LegacyIdentityResolution::Unavailable);
    };
    let selected =
        gguf_selection::select_primary_gguf(parent)?.and_then(|path| path.canonicalize().ok());
    if selected.as_deref() != Some(canonical_path.as_path()) {
        return Ok(LegacyIdentityResolution::Unavailable);
    }
    identity_for_weight_path(&canonical_path).map(LegacyIdentityResolution::Unique)
}

fn resolve_opaque_identity(model_root: &Path) -> Result<LegacyIdentityResolution, GemmaError> {
    let ready_models = super::scan_models(model_root)?
        .into_iter()
        .filter(|model| model.format == "gguf" && model.compatibility == "ready")
        .map(|model| identity_for_model_directory(Path::new(&model.path)))
        .collect::<Result<Vec<_>, _>>()?;
    unique_identity(ready_models)
}

fn resolve_named_identity(
    model_root: &Path,
    reference: &str,
) -> Result<LegacyIdentityResolution, GemmaError> {
    let models = super::scan_models(model_root)?;
    let canonical_reference = canonical_registry_match(reference)
        .or_else(|| (normalize(reference) == "gemma42b").then_some(GEMMA_E2B_CANONICAL_ID));
    let matches = models
        .into_iter()
        .filter(|model| model.format == "gguf" && model.compatibility == "ready")
        .filter(|model| {
            model.id.eq_ignore_ascii_case(reference)
                || canonical_reference.is_some_and(|canonical| model.id == canonical)
                || model_directory_matches_legacy_reference(Path::new(&model.path), reference)
        })
        .map(|model| identity_for_model_directory(Path::new(&model.path)))
        .collect::<Result<Vec<_>, _>>()?;
    unique_identity(matches)
}

fn unique_identity(
    mut matches: Vec<LocalModelIdentity>,
) -> Result<LegacyIdentityResolution, GemmaError> {
    matches.sort_by(|left, right| left.canonical_id.cmp(&right.canonical_id));
    matches.dedup_by(|left, right| left.canonical_id == right.canonical_id);
    match matches.len() {
        0 => Ok(LegacyIdentityResolution::Unavailable),
        1 => Ok(LegacyIdentityResolution::Unique(matches.remove(0))),
        _ => Ok(LegacyIdentityResolution::Ambiguous),
    }
}

pub fn canonical_registry_match(candidate: &str) -> Option<&'static str> {
    let leaf = candidate
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(candidate)
        .strip_suffix(".gguf")
        .unwrap_or_else(|| {
            candidate
                .trim()
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(candidate)
        });
    let normalized = normalize(leaf);
    for (prefix, canonical_id) in [
        ("gemma4e2b", GEMMA_E2B_CANONICAL_ID),
        ("gemma4e4b", GEMMA_E4B_CANONICAL_ID),
        ("gemma412b", GEMMA_12B_CANONICAL_ID),
    ] {
        if normalized
            .strip_prefix(prefix)
            .is_some_and(is_supported_variant_suffix)
        {
            return Some(canonical_id);
        }
    }
    None
}

pub fn is_opaque_storage_reference(value: &str) -> bool {
    let basename = value
        .trim()
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    matches!(
        basename.as_str(),
        "models" | "model" | "local-model" | "local_models"
    )
}

pub fn canonical_display_name(canonical_id: &str) -> String {
    match canonical_id {
        GEMMA_E2B_CANONICAL_ID => GEMMA_E2B_FULL_DISPLAY_NAME.to_string(),
        GEMMA_E4B_CANONICAL_ID => "Gemma 4 E4B".to_string(),
        GEMMA_12B_CANONICAL_ID => "Gemma 4 12B".to_string(),
        other => local_model_label(other),
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_supported_variant_suffix(suffix: &str) -> bool {
    let mut remaining = suffix;
    while !remaining.is_empty() {
        let Some(part) = [
            "instruct", "gguf", "chat", "qat", "q4km", "q40", "q6k", "q80", "it",
        ]
        .iter()
        .find(|part| remaining.starts_with(**part)) else {
            return false;
        };
        remaining = &remaining[part.len()..];
    }
    true
}

fn identity_for_weight_path(weight_path: &Path) -> Result<LocalModelIdentity, GemmaError> {
    let parent = weight_path.parent().ok_or_else(|| GemmaError {
        code: "local_model_identity_ambiguous",
        message: "OOMU could not verify a stable identity for this local model.".to_string(),
    })?;
    let stem = weight_path
        .file_stem()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| GemmaError {
            code: "local_model_identity_ambiguous",
            message: "OOMU could not verify a stable identity for this local model.".to_string(),
        })?;
    if let Some(canonical_id) = canonical_registry_match(stem) {
        return Ok(LocalModelIdentity {
            canonical_id: canonical_id.to_string(),
            display_name: canonical_display_name(canonical_id),
            storage_directory: parent.to_path_buf(),
            source: LocalModelIdentitySource::CanonicalRegistry,
        });
    }
    if let Ok(metadata) = gguf_identity_metadata(weight_path) {
        if let Some(canonical_id) = canonical_registry_match(&metadata.name) {
            return Ok(LocalModelIdentity {
                canonical_id: canonical_id.to_string(),
                display_name: canonical_display_name(canonical_id),
                storage_directory: parent.to_path_buf(),
                source: LocalModelIdentitySource::CanonicalRegistry,
            });
        }
        return Ok(metadata_identity(parent, metadata));
    }
    let canonical_id = stable_custom_id(stem);
    Ok(LocalModelIdentity {
        display_name: local_model_label(&canonical_id),
        canonical_id,
        storage_directory: parent.to_path_buf(),
        source: LocalModelIdentitySource::WeightMetadata,
    })
}

fn stable_custom_id(value: &str) -> String {
    let mut id = String::with_capacity(value.len());
    let mut previous_separator = false;
    for character in value.trim().chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_') {
            id.push(character);
            previous_separator = false;
        } else if !previous_separator {
            id.push('-');
            previous_separator = true;
        }
    }
    id.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parser_valid_gguf_fixture() -> Option<PathBuf> {
        let mut registries = Vec::new();
        if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
            registries.push(PathBuf::from(cargo_home).join("registry/src"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            registries.push(PathBuf::from(home).join(".cargo/registry/src"));
        }
        registries
            .into_iter()
            .flat_map(|registry| fs::read_dir(registry).ok().into_iter().flatten())
            .filter_map(Result::ok)
            .map(|entry| {
                entry
                    .path()
                    .join("llama-cpp-2-0.1.151/src/gguf/ggml-vocab-bert-bge.gguf")
            })
            .find(|candidate| candidate.is_file())
    }

    fn temporary_model_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oomu-{label}-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ))
    }

    #[test]
    fn canonical_registry_recognizes_supported_gemma_storage_aliases() {
        assert_eq!(
            canonical_registry_match("gemma-4-E2B_q4_0-it"),
            Some(GEMMA_E2B_CANONICAL_ID)
        );
        assert_eq!(
            canonical_registry_match("google/gemma-4-E4B-it"),
            Some(GEMMA_E4B_CANONICAL_ID)
        );
        assert_eq!(
            canonical_registry_match("gemma-4-12b-it-qat-q4_0"),
            Some(GEMMA_12B_CANONICAL_ID)
        );
    }

    #[test]
    fn valid_custom_gguf_identity_survives_file_and_folder_renames() {
        let Some(fixture) = parser_valid_gguf_fixture() else {
            return;
        };
        let root = temporary_model_root("metadata-identity-rename");
        let first = root.join("publisher-layout");
        let second = root.join("renamed-by-user");
        fs::create_dir_all(&first).expect("create first model directory");
        fs::create_dir_all(&second).expect("create renamed model directory");
        fs::copy(&fixture, first.join("original-release-name.gguf"))
            .expect("copy parser-valid GGUF fixture");
        fs::copy(&fixture, second.join("my-preferred-name.gguf"))
            .expect("copy renamed parser-valid GGUF fixture");

        let original = identity_for_model_directory(&first).expect("identify original model");
        let renamed = identity_for_model_directory(&second).expect("identify renamed model");

        assert_eq!(original.canonical_id, renamed.canonical_id);
        assert_eq!(original.display_name, renamed.display_name);
        assert!(original.canonical_id.starts_with("gguf-bert-"));
        assert_eq!(original.source, LocalModelIdentitySource::WeightMetadata);
        assert!(!original.display_name.contains("original-release-name"));
        assert!(!renamed.display_name.contains("my-preferred-name"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn valid_custom_gguf_identity_ignores_mutable_config_labels() {
        let Some(fixture) = parser_valid_gguf_fixture() else {
            return;
        };
        let root = temporary_model_root("metadata-identity-config");
        let model_dir = root.join("model");
        fs::create_dir_all(&model_dir).expect("create model directory");
        fs::copy(&fixture, model_dir.join("weights.gguf")).expect("copy parser-valid GGUF fixture");
        fs::write(
            model_dir.join(MODEL_CONFIG),
            serde_json::json!({"_name_or_path": "First user label"}).to_string(),
        )
        .expect("write first model label");
        let first = identity_for_model_directory(&model_dir).expect("identify first label");
        fs::write(
            model_dir.join(MODEL_CONFIG),
            serde_json::json!({"_name_or_path": "Renamed user label"}).to_string(),
        )
        .expect("write renamed model label");
        let renamed = identity_for_model_directory(&model_dir).expect("identify renamed label");

        assert_eq!(first.canonical_id, renamed.canonical_id);
        assert_eq!(first.display_name, renamed.display_name);
        assert!(model_directory_matches_legacy_reference(
            &model_dir,
            "Renamed-user-label"
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canonical_e2b_identity_uses_the_fully_qualified_product_name() {
        assert_eq!(
            canonical_display_name(GEMMA_E2B_CANONICAL_ID),
            GEMMA_E2B_FULL_DISPLAY_NAME
        );
    }

    #[test]
    fn legacy_default_reference_repairs_to_installed_e2b_without_using_a_directory_name() {
        let root =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        if !root.join(GEMMA_E2B_CANONICAL_ID).is_dir() {
            return;
        }
        let resolved = resolve_legacy_identity(&root, "gemma-4-2b")
            .expect("legacy default reference resolves");
        let LegacyIdentityResolution::Unique(identity) = resolved else {
            panic!("the installed E2B model makes the legacy default unambiguous");
        };
        assert_eq!(identity.canonical_id, GEMMA_E2B_CANONICAL_ID);
    }

    #[test]
    fn storage_directory_names_are_never_treated_as_model_identity() {
        for value in ["models", "Models", "/private/test/models/"] {
            assert!(is_opaque_storage_reference(value));
            assert_eq!(canonical_registry_match(value), None);
        }
    }

    #[test]
    fn canonical_registry_rejects_embedded_family_substrings() {
        for value in [
            "not-gemma-4-E2B-it",
            "gemma-4-E4B-impersonator",
            "archive-gemma-4-12B-it-backup",
        ] {
            assert_eq!(canonical_registry_match(value), None);
        }
    }

    #[test]
    fn a_directory_name_alone_cannot_establish_model_identity() {
        let root = std::env::temp_dir().join(format!(
            "oomu-model-identity-name-only-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        let directory = root.join(GEMMA_E2B_CANONICAL_ID);
        std::fs::create_dir_all(&directory).expect("empty named directory exists");
        let error = identity_for_model_directory(&directory)
            .expect_err("storage basename alone is not identity evidence");
        assert_eq!(error.code, "local_model_identity_ambiguous");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn full_primary_gguf_path_resolves_to_canonical_identity() {
        let root =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let weight = root
            .join(GEMMA_E2B_CANONICAL_ID)
            .join("gemma-4-E2B_q4_0-it.gguf");
        if !weight.is_file() {
            return;
        }
        let resolved = resolve_legacy_identity(&root, weight.to_string_lossy().as_ref())
            .expect("full GGUF path resolves");
        let LegacyIdentityResolution::Unique(identity) = resolved else {
            panic!("full primary GGUF path must resolve uniquely");
        };
        assert_eq!(identity.canonical_id, GEMMA_E2B_CANONICAL_ID);
        assert_eq!(
            identity.storage_directory,
            weight.parent().unwrap().canonicalize().unwrap()
        );
    }
}
