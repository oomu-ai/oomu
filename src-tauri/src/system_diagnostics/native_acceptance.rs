use crate::db::{ChatTurnPersistenceContext, PersistenceEngine};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemDiagnosticsRequest {
    #[serde(default = "super::default_true")]
    pub include_memory_audit: bool,
    #[serde(default = "super::default_true")]
    pub include_pre_alpha_audit: bool,
    #[serde(default = "super::default_true")]
    pub export_markdown: bool,
    pub pre_alpha_runs: Option<usize>,
    pub memory_query: Option<String>,
    #[serde(default)]
    pub memory_channels: Vec<String>,
    pub minimum_memory_recurrence: Option<usize>,
    #[serde(default)]
    pub turn_context: Option<SystemDiagnosticsTurnContext>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SystemDiagnosticsTurnContext {
    pub turn_id: String,
    pub generation_token: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub parent_turn_id: Option<String>,
    pub root_turn_id: String,
    pub turn_kind: String,
}

impl From<&SystemDiagnosticsTurnContext> for ChatTurnPersistenceContext {
    fn from(value: &SystemDiagnosticsTurnContext) -> Self {
        Self {
            turn_id: value.turn_id.trim().to_string(),
            generation_token: value.generation_token.trim().to_string(),
            session_id: value.session_id.trim().to_string(),
            agent_id: value.agent_id.trim().to_string(),
            provider_id: value.provider_id.trim().to_string(),
            model_id: value.model_id.trim().to_string(),
            parent_turn_id: value
                .parent_turn_id
                .as_deref()
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string),
            root_turn_id: value.root_turn_id.trim().to_string(),
            turn_kind: value.turn_kind.trim().to_string(),
        }
    }
}

pub(super) async fn record_full_disk_access_probe(
    request: &super::SystemDiagnosticsRequest,
    persistence: &PersistenceEngine,
) -> Result<(), String> {
    let Some(turn_context) = request.turn_context.as_ref() else {
        return Ok(());
    };
    let context = ChatTurnPersistenceContext::from(turn_context);
    persistence
        .ensure_chat_turn_for_native_action(&context)
        .map_err(|_| "System diagnostics could not verify the active chat turn.".to_string())?;
    let action_binding = format!(
        "system-diagnostics-fda:{}",
        crate::foundation::digest::sha256_hex(
            format!(
                "{}:{}:{}",
                context.session_id, context.turn_id, context.generation_token
            )
            .as_bytes(),
        )
    );
    let Some(attempt) = crate::tools::native_operation_receipt::NativeOperationAttempt::begin(
        crate::tools::native_operation_receipt::AppleCapability::FullDiskAccess,
        crate::tools::native_operation_receipt::NativeActionClass::Read,
        true,
        action_binding,
        Some(&context),
    )
    .await
    else {
        return Err("System diagnostics could not bind its permission check.".to_string());
    };
    let evidence = full_disk_access_probe_evidence(
        crate::native_capability_adapters::probe_full_disk_access(),
    );
    let _ = attempt.finish(evidence).await;
    Ok(())
}

fn full_disk_access_probe_evidence(
    probe: crate::native_capability_adapters::FullDiskAccessProbe,
) -> crate::tools::native_operation_receipt::NativePostconditionEvidence {
    use crate::native_capability_adapters::FullDiskAccessProbe;
    let (operation_succeeded, verified, bounded_count, result_code) = match probe {
        FullDiskAccessProbe::Allowed { bytes_read } => (
            true,
            bytes_read == 16,
            Some(bytes_read as u64),
            "bounded_probe_allowed",
        ),
        FullDiskAccessProbe::PermissionRequired => (true, true, None, "permission_required"),
        FullDiskAccessProbe::Stale => (false, false, None, "probe_stale"),
        FullDiskAccessProbe::Unsupported => (false, false, None, "probe_unsupported"),
    };
    crate::tools::native_operation_receipt::NativePostconditionEvidence {
        evidence_kind: "bounded_full_disk_access_probe",
        operation_succeeded,
        verified,
        bounded_count,
        truncated: Some(false),
        native_result_code: Some(result_code.to_string()),
        durable_operation_binding: None,
        capture_proof: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_proves_only_the_bounded_probe() {
        let allowed = full_disk_access_probe_evidence(
            crate::native_capability_adapters::FullDiskAccessProbe::Allowed { bytes_read: 16 },
        );
        assert_eq!(allowed.evidence_kind, "bounded_full_disk_access_probe");
        assert!(allowed.operation_succeeded);
        assert!(allowed.verified);
        assert_eq!(allowed.bounded_count, Some(16));

        let denied = full_disk_access_probe_evidence(
            crate::native_capability_adapters::FullDiskAccessProbe::PermissionRequired,
        );
        assert!(denied.operation_succeeded);
        assert!(denied.verified);
        assert_eq!(denied.bounded_count, None);
        assert_eq!(
            denied.native_result_code.as_deref(),
            Some("permission_required")
        );
    }

    #[test]
    fn short_read_never_claims_verification() {
        let evidence = full_disk_access_probe_evidence(
            crate::native_capability_adapters::FullDiskAccessProbe::Allowed { bytes_read: 8 },
        );
        assert!(evidence.operation_succeeded);
        assert!(!evidence.verified);
        assert_eq!(evidence.bounded_count, Some(8));
    }
}
