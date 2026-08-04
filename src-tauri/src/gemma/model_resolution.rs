use super::*;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupModelSelectionSource {
    CleanDefault,
    ExplicitUserSelection,
}

impl StartupModelSelectionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CleanDefault => "clean_default",
            Self::ExplicitUserSelection => "explicit_user_selection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupModelPreference {
    pub requested_model_id: String,
    pub selection_source: StartupModelSelectionSource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartupModelAssignment {
    pub requested_model_id: String,
    pub resolved_model_id: String,
    pub resolved_directory: PathBuf,
    pub selection_source: StartupModelSelectionSource,
    pub identity: LocalModelIdentity,
}

impl GemmaService {
    pub(super) fn load_model_from_dir_with_context(
        &self,
        model_dir: PathBuf,
        min_context_size: Option<u32>,
    ) -> Result<(), GemmaError> {
        let _load_guard = self.lock_model_load();
        self.load_model_from_dir_with_context_locked(model_dir, min_context_size)
    }

    pub(super) fn load_model_from_dir_with_context_locked(
        &self,
        model_dir: PathBuf,
        min_context_size: Option<u32>,
    ) -> Result<(), GemmaError> {
        let result = (|| {
            if let Some(gguf_path) = gguf_selection::select_primary_gguf(&model_dir)? {
                return self.load_gguf_model(model_dir, gguf_path, min_context_size);
            }
            Err(GemmaError {
                code: "local_infer_stateful_gguf_required",
                message: "No valid GGUF asset was found in the configured local-model store. OOMU local inference accepts GGUF models only; configure or download a quantized GGUF model."
                    .to_string(),
            })
        })();
        if let Err(error) = &result {
            self.enter_local_generation_degraded(error.clone());
        }
        result
    }

    pub(super) fn ensure_requested_context_capacity(
        &self,
        requested_context_size: Option<u32>,
    ) -> Result<(), GemmaError> {
        let _load_guard = self.lock_model_load();
        let Some(required_context_size) =
            requested_context_size.map(|size| size.clamp(512, 131_072))
        else {
            return Ok(());
        };
        let model_to_reload = {
            let mut state = self.lock_state();
            let Some(model) = state.model.as_ref() else {
                return Ok(());
            };
            if model.profile.runtime_config.context_size >= required_context_size {
                return Ok(());
            }
            let model_dir = model.model_dir.clone();
            state.status = GemmaStatus::Loading;
            state.degraded_reason = None;
            let previous_model = state.model.take();
            drop(state);
            drop(previous_model);
            model_dir
        };
        self.load_model_from_dir_with_context_locked(model_to_reload, Some(required_context_size))
    }
}

pub(super) fn resolve_configured_local_model(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    let requested_model_id = requested_model_id.trim();
    if let Ok(model_dir) = local_model_dir_under_root(model_root, requested_model_id) {
        if let Ok(model) = inspect_local_model_directory(&model_dir, requested_model_id) {
            if is_ready_gguf(&model) {
                return Ok(model);
            }
        }
    }
    let models = scan_models(model_root)?;
    if is_legacy_default_model_alias(requested_model_id) {
        if let Some(model) = select_preferred_ready_chat_gguf(&models) {
            return Ok(model);
        }
    }
    if let Some(model) = select_same_family_ready_gguf(&models, requested_model_id) {
        return Ok(model);
    }
    Err(GemmaError {
        code: "configured_local_model_unavailable",
        message: format!(
            "The configured local model '{requested_model_id}' is unavailable; OOMU will not substitute a different model family."
        ),
    })
}

pub(crate) fn resolve_canonical_ready_local_model(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    let requested_model_id = requested_model_id.trim();
    // Exact model verification is intentionally backed by the shared,
    // presence-checked resolution cache. Readiness is polled while Chat is
    // visible; bypassing this cache re-opened the multi-gigabyte GGUF on every
    // poll and allowed concurrent model inspections to starve unrelated IPC.
    let cache_key = (model_root.to_path_buf(), requested_model_id.to_string());
    let cache = LOCAL_MODEL_RESOLUTION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(model) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&cache_key)
        .filter(|model| is_ready_gguf(model) && cached_local_model_is_present(model))
        .cloned()
    {
        return require_exact_model_identity(model, requested_model_id);
    }

    let model = require_exact_model_identity(
        resolve_configured_local_model(model_root, requested_model_id)?,
        requested_model_id,
    )?;
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(cache_key, model.clone());
    Ok(model)
}

fn require_exact_model_identity(
    model: LocalModelOption,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    if model.id.eq_ignore_ascii_case(requested_model_id) {
        return Ok(model);
    }
    Err(GemmaError {
        code: "local_model_identity_mismatch",
        message: "The requested on-device model does not match the verified installed model."
            .to_string(),
    })
}

#[cfg(test)]
fn resolve_startup_local_model_assignment(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<Option<LocalModelOption>, GemmaError> {
    match resolve_configured_local_model(model_root, requested_model_id) {
        Ok(model) => Ok(Some(model)),
        Err(error) if error.code == "configured_local_model_unavailable" => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn resolve_verified_startup_model_assignment(
    model_root: &Path,
    preference: &StartupModelPreference,
) -> Result<StartupModelAssignment, GemmaError> {
    let resolved = resolve_canonical_ready_local_model(model_root, &preference.requested_model_id)?;
    let identity = identity_for_model_directory(Path::new(&resolved.path))?;
    Ok(StartupModelAssignment {
        requested_model_id: preference.requested_model_id.clone(),
        resolved_model_id: identity.canonical_id.clone(),
        resolved_directory: identity.storage_directory.clone(),
        selection_source: preference.selection_source,
        identity,
    })
}

pub(super) fn resolve_local_model_uncached(
    model_root: &Path,
    requested_model_id: &str,
) -> Result<LocalModelOption, GemmaError> {
    if let Ok(model_dir) = local_model_dir_under_root(model_root, requested_model_id) {
        if let Ok(model) = inspect_local_model_directory(&model_dir, requested_model_id) {
            if is_ready_gguf(&model) {
                return Ok(model);
            }
            eprintln!(
                "LOCAL_MODEL_GGUF_REQUIRED requested_model_id={} detected_format={} compatibility={} message={}",
                requested_model_id,
                model.format,
                model.compatibility,
                model.compatibility_message
            );
        }
    }

    let models = scan_models(model_root)?;
    if is_legacy_default_model_alias(requested_model_id) {
        if let Some(model) = select_preferred_ready_chat_gguf(&models) {
            eprintln!(
                "LOCAL_MODEL_LEGACY_DEFAULT_MIGRATION requested_model_id={} resolved_model_id={}",
                requested_model_id, model.id
            );
            return Ok(model);
        }
    }
    if let Some(model) = select_same_family_ready_gguf(&models, requested_model_id) {
        eprintln!(
            "LOCAL_MODEL_SAME_FAMILY_ALIAS requested_model_id={} resolved_model_id={}",
            requested_model_id, model.id
        );
        return Ok(model);
    }

    if let Ok(preferred_dir) = local_model_dir_under_root(model_root, PREFERRED_LOCAL_MODEL_ID) {
        if let Ok(model) = inspect_local_model_directory(&preferred_dir, PREFERRED_LOCAL_MODEL_ID) {
            if is_ready_chat_gguf(&model) {
                return Ok(model);
            }
        }
    }

    eprintln!(
        "LOCAL_MODEL_STATEFUL_GGUF_FALLBACK model_store=private_local_model requested_id_bytes={} preferred_model_id={}",
        requested_model_id.len(),
        PREFERRED_LOCAL_MODEL_ID
    );
    select_best_ready_gguf(&models).ok_or_else(|| GemmaError {
        code: "local_model_fallback_unavailable",
        message: "The requested local model is unavailable or incompatible, and no ready GGUF fallback exists in the configured local-model store."
            .to_string(),
    })
}

fn is_legacy_default_model_alias(model_id: &str) -> bool {
    model_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .eq("gemma42b".chars())
}

fn select_preferred_ready_chat_gguf(models: &[LocalModelOption]) -> Option<LocalModelOption> {
    models
        .iter()
        .find(|model| model.id == PREFERRED_LOCAL_MODEL_ID && is_ready_chat_gguf(model))
        .cloned()
}

fn local_model_family(model_id: &str) -> Option<&'static str> {
    let normalized = model_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if !normalized.contains("gemma4") {
        return None;
    }
    if normalized.contains("e2b") {
        Some("gemma4-e2b")
    } else if normalized.contains("e4b") {
        Some("gemma4-e4b")
    } else if normalized.contains("12b") {
        Some("gemma4-12b")
    } else if normalized.contains("2b") {
        Some("gemma4-e2b")
    } else {
        None
    }
}

fn select_same_family_ready_gguf(
    models: &[LocalModelOption],
    requested_model_id: &str,
) -> Option<LocalModelOption> {
    let family = local_model_family(requested_model_id)?;
    models
        .iter()
        .filter(|model| {
            is_ready_gguf(model) && local_model_family(&model.id).is_some_and(|id| id == family)
        })
        .max_by_key(|model| (model.chat_capability == "chat", model.weights_bytes))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_default_alias_is_not_an_explicit_e2b_or_12b_selection() {
        assert!(is_legacy_default_model_alias("gemma-4-2b"));
        assert!(!is_legacy_default_model_alias(
            "gemma-4-E2B-it-qat-q4_0-gguf"
        ));
        assert!(!is_legacy_default_model_alias(
            "gemma-4-12B-it-qat-q4_0-gguf"
        ));
    }

    #[test]
    fn quantization_alias_preserves_the_configured_12b_model_family() {
        let root =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let installed_12b = "gemma-4-12B-it-qat-q4_0-gguf";
        if !root.join(installed_12b).is_dir() {
            return;
        }

        let resolved = resolve_configured_local_model(&root, "gemma-4-12B-it-q8_0-gguf")
            .expect("resolve the installed quantization from the configured 12B family");
        assert_eq!(resolved.id, installed_12b);
        assert_eq!(local_model_family(&resolved.id), Some("gemma4-12b"));
        assert_ne!(resolved.id, PREFERRED_LOCAL_MODEL_ID);
        assert_eq!(
            local_model_family("gemma-4-12B-it-q8_0-gguf"),
            local_model_family(installed_12b)
        );
        assert_ne!(
            local_model_family("gemma-4-12B-it-q8_0-gguf"),
            local_model_family(PREFERRED_LOCAL_MODEL_ID)
        );
    }

    #[cfg(unix)]
    #[test]
    fn strict_configured_resolution_rejects_cross_family_fallback() {
        let assets =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let installed_preferred = assets.join(PREFERRED_LOCAL_MODEL_ID);
        if !installed_preferred.is_dir() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "oomu-strict-model-family-{}-{}",
            std::process::id(),
            unix_time_ns()
        ));
        fs::create_dir_all(&root).expect("create strict model test root");
        std::os::unix::fs::symlink(&installed_preferred, root.join(PREFERRED_LOCAL_MODEL_ID))
            .expect("link installed preferred-model fixture");

        let error = resolve_configured_local_model(&root, "gemma-4-12B-it-q8_0-gguf")
            .expect_err("strict 12B selection must not substitute the available preferred model");
        assert_eq!(error.code, "configured_local_model_unavailable");
        assert!(error.message.contains("will not substitute"));
        assert!(
            resolve_startup_local_model_assignment(&root, "gemma-4-12B-it-q8_0-gguf")
                .expect("startup assignment resolution remains available")
                .is_none()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn unavailable_e2b_never_falls_back_to_e4b_or_cloud() {
        let assets =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let e4b_id = "gemma-4-E4B-it-qat-q4_0-gguf";
        if !assets.join(e4b_id).is_dir() {
            return;
        }
        let root = std::env::temp_dir().join(format!(
            "oomu-startup-e2b-strict-{}-{}",
            std::process::id(),
            unix_time_ns()
        ));
        fs::create_dir_all(&root).expect("create strict startup model root");
        std::os::unix::fs::symlink(assets.join(e4b_id), root.join(e4b_id))
            .expect("link the real installed E4B model");
        let preference = StartupModelPreference {
            requested_model_id: CLEAN_INSTALL_STARTUP_MODEL_ID.to_string(),
            selection_source: StartupModelSelectionSource::CleanDefault,
        };

        let error = resolve_verified_startup_model_assignment(&root, &preference)
            .expect_err("an unavailable E2B assignment must stop without substitution");
        assert_eq!(error.code, "configured_local_model_unavailable");
        assert!(!error.message.contains(e4b_id));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_root_model_identity_is_not_directory_basename() {
        let assets =
            PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join("../assets/models");
        let installed = assets.join(PREFERRED_LOCAL_MODEL_ID);
        if !installed.is_dir() {
            return;
        }
        let parent = std::env::temp_dir().join(format!(
            "oomu-canonical-root-model-{}-{}",
            std::process::id(),
            unix_time_ns()
        ));
        let root = parent.join("models");
        fs::create_dir_all(&parent).expect("create canonical root parent");
        std::os::unix::fs::symlink(&installed, &root).expect("link E2B as root model store");

        let discovered = scan_models(&root).expect("discover root-level model");
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id, PREFERRED_LOCAL_MODEL_ID);
        assert_ne!(discovered[0].id, "models");

        let resolved = resolve_configured_local_model(&root, "models")
            .expect("legacy storage basename resolves only through verified root metadata");
        assert_eq!(resolved.id, PREFERRED_LOCAL_MODEL_ID);
        let _ = fs::remove_file(root);
        let _ = fs::remove_dir_all(parent);
    }

    #[test]
    fn canonical_root_model_coordinates_resolve_to_the_store_itself() {
        let root = std::env::temp_dir().join(format!(
            "oomu-root-level-e2b-{}-{}",
            std::process::id(),
            unix_time_ns()
        ));
        fs::create_dir_all(&root).expect("create root-level model store");
        fs::write(root.join("gemma-4-E2B_q4_0-it.gguf"), b"GGUF")
            .expect("write root-level model identity evidence");

        let resolved = local_model_dir_under_root(&root, GEMMA_E2B_CANONICAL_ID)
            .expect("resolve root-level canonical model identity");

        assert_eq!(resolved, root);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn canonical_ready_resolution_rejects_cross_identity_substitution() {
        let model = LocalModelOption {
            name: "Gemma 4 E4B".to_string(),
            id: GEMMA_E4B_CANONICAL_ID.to_string(),
            path: "/private/test/models/e4b".to_string(),
            weights_bytes: 1,
            format: "gguf".to_string(),
            architecture: "gemma4".to_string(),
            compatibility: "ready".to_string(),
            compatibility_message: "verified".to_string(),
            chat_capability: "chat".to_string(),
        };

        let error = require_exact_model_identity(model, GEMMA_E2B_CANONICAL_ID)
            .expect_err("cross-identity substitution must fail closed");

        assert_eq!(error.code, "local_model_identity_mismatch");
    }
}
