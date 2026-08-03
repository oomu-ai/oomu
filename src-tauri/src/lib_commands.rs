pub(crate) mod degraded_mode_commands {
    use crate::{
        sovereign_identity, BackingStoreClass, DegradedModeState, DegradedModeStatus,
        VolatileStoreSessionManager,
    };

    #[tauri::command]
    pub fn get_degraded_mode_status(
        degraded_mode: tauri::State<'_, DegradedModeState>,
        sessions: tauri::State<'_, VolatileStoreSessionManager>,
    ) -> DegradedModeStatus {
        let mut status = degraded_mode.snapshot();
        let has_recovery_session = sessions
            .current()
            .is_some_and(|session| session.root().exists());
        if has_recovery_session {
            if !status.subsystems.iter().any(|subsystem| {
                subsystem.subsystem == "chatSessionPersistence" && subsystem.active
            }) {
                degraded_mode.activate(
                    "chatSessionPersistence",
                    "Private encrypted volatile recovery data exists and requires reconciliation or export.",
                    crate::BackingStoreClass::RecoveryPending,
                    true,
                    "Chat/session recovery data remains pending; durable health cannot be reported until it is reconciled and explicitly cleaned up.",
                );
                status = degraded_mode.snapshot();
            }
            status.has_volatile_storage = true;
        }
        status
    }

    #[tauri::command]
    pub fn retry_sovereign_identity_health(
        identity: tauri::State<'_, sovereign_identity::SovereignIdentity>,
        degraded_mode: tauri::State<'_, DegradedModeState>,
    ) -> Result<sovereign_identity::IdentityProfile, sovereign_identity::IdentityError> {
        match identity.retry_secure_storage_probe() {
            Ok(profile) => {
                degraded_mode.clear_after_verified_recovery(
                    "identity",
                    BackingStoreClass::Persistent,
                    "Secure identity and OS Keychain probes succeeded after an explicit retry.",
                );
                Ok(profile)
            }
            Err(error) => {
                degraded_mode.activate(
                    "identity",
                    format!("Secure identity health probe failed: {}", error.message),
                    BackingStoreClass::RecoveryPending,
                    true,
                    "Signing and identity-backed operations are unavailable.",
                );
                Err(error)
            }
        }
    }
}

pub(crate) mod local_inference_recovery_commands {
    use crate::{
        gemma, inference, settings, BackingStoreClass, DegradedModeState, DegradedModeStatus,
    };
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct LocalInferenceRecoveryResponse {
        pub model_id: String,
        pub model_name: String,
        pub degraded_mode: DegradedModeStatus,
    }

    fn recovery_error(code: &'static str, message: impl Into<String>) -> gemma::GemmaError {
        gemma::GemmaError {
            code,
            message: message.into(),
        }
    }

    pub(super) fn require_verified_worker_prewarm(
        result: Result<(), inference::InferenceError>,
    ) -> Result<(), gemma::GemmaError> {
        result.map_err(|error| {
            recovery_error(
                "local_inference_worker_prewarm_failed",
                format!(
                    "Local inference worker prewarm failed ({} at {}): {}",
                    error.code, error.boundary, error.message
                ),
            )
        })
    }

    #[cfg(test)]
    mod recovery_boundary_tests {
        use super::require_verified_worker_prewarm;
        use crate::*;

        #[test]
        fn local_inference_recovery_preserves_worker_failure_without_classifier_mutation() {
            let error = require_verified_worker_prewarm(Err(inference::InferenceError {
                code: "local_worker_probe_failed".to_string(),
                boundary: "local_inference_worker".to_string(),
                message: "generation worker did not answer its readiness probe".to_string(),
            }))
            .expect_err("failed worker prewarm remains a local-generation failure");
            assert_eq!(error.code, "local_inference_worker_prewarm_failed");
            assert!(error.message.contains("local_worker_probe_failed"));
        }
    }

    #[tauri::command]
    pub async fn recover_local_inference(
        model_id: String,
        app: tauri::AppHandle,
        degraded_mode: tauri::State<'_, DegradedModeState>,
    ) -> Result<LocalInferenceRecoveryResponse, gemma::GemmaError> {
        let requested_model_id = model_id.trim().to_string();
        let model_root = settings::resolved_local_model_directory(&app)
            .map_err(|message| recovery_error("local_model_directory_unavailable", message))?;
        let probe_root = model_root.clone();
        let selected_model = match tauri::async_runtime::spawn_blocking(move || {
            let selected =
                gemma::resolve_exact_ready_local_model(&probe_root, &requested_model_id)?;
            require_verified_worker_prewarm(inference::prewarm_local_inference_worker(
                &selected.id,
                &probe_root,
            ))?;
            Ok::<_, gemma::GemmaError>(selected)
        })
        .await
        {
            Ok(result) => result,
            Err(join_error) => {
                let error = recovery_error(
                    "local_inference_recovery_worker_failed",
                    format!("Local inference recovery worker failed: {join_error}"),
                );
                degraded_mode.activate(
                    "inference",
                    format!("Local inference recovery failed: {}", error.message),
                    BackingStoreClass::NotApplicable,
                    true,
                    "Local model generation is unavailable until a model probe succeeds.",
                );
                return Err(error);
            }
        };
        let selected_model = match selected_model {
            Ok(selected_model) => selected_model,
            Err(error) => {
                degraded_mode.activate(
                    "inference",
                    format!("Local inference recovery failed: {}", error.message),
                    BackingStoreClass::NotApplicable,
                    true,
                    "Local model generation is unavailable until a model probe succeeds.",
                );
                return Err(error);
            }
        };

        if let Err(message) =
            settings::set_default_prewarmed_model(app, selected_model.id.clone()).await
        {
            let error = recovery_error("default_prewarmed_model_save_failed", message);
            degraded_mode.activate(
                "inference",
                format!(
                    "The selected local model loaded, but its startup preference could not be saved: {}",
                    error.message
                ),
                BackingStoreClass::NotApplicable,
                true,
                "Local model generation is loaded for this run, but verified startup recovery remains pending.",
            );
            return Err(error);
        }

        degraded_mode.clear_after_verified_recovery(
            "inference",
            BackingStoreClass::NotApplicable,
            format!(
                "Exact selected local model '{}' loaded and its startup preference was saved.",
                selected_model.id
            ),
        );

        Ok(LocalInferenceRecoveryResponse {
            model_id: selected_model.id,
            model_name: selected_model.name,
            degraded_mode: degraded_mode.snapshot(),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::require_verified_worker_prewarm;
        use crate::*;

        #[test]
        fn worker_prewarm_failure_blocks_verified_inference_health() {
            let error = require_verified_worker_prewarm(Err(inference::InferenceError {
                code: "worker_start_failed".to_string(),
                boundary: "local_inference_worker".to_string(),
                message: "native worker did not start".to_string(),
            }))
            .expect_err("worker prewarm failure must prevent verified inference health");

            assert_eq!(error.code, "local_inference_worker_prewarm_failed");
            assert!(error.message.contains("worker_start_failed"));
            assert!(error.message.contains("native worker did not start"));
        }

        #[test]
        fn worker_prewarm_success_allows_verified_inference_health() {
            require_verified_worker_prewarm(Ok(()))
                .expect("successful worker prewarm should allow verified inference health");
        }
    }
}

pub(crate) mod launch_options_commands {
    use crate::OomuLaunchOptions;

    #[tauri::command]
    pub fn get_launch_options(
        launch_options: tauri::State<'_, OomuLaunchOptions>,
    ) -> OomuLaunchOptions {
        launch_options.inner().clone()
    }
}
