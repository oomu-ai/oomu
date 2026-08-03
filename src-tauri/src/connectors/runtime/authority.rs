use super::super::{adapter, repository, ConnectorCapabilityGrant};
use super::known_capability;
use crate::db::PersistenceEngine;

#[derive(Clone, Copy, Debug)]
pub(super) struct ValidatedConnectorAuthority {
    pub operation: &'static str,
}

pub(super) fn executable_capabilities(
    grants: &[ConnectorCapabilityGrant],
    registered: &dyn adapter::ConnectorAdapter,
) -> Vec<&'static str> {
    let mut capabilities = grants
        .iter()
        .filter(|grant| grant.available && grant.granted)
        .flat_map(|grant| registered.capabilities_for_operation(&grant.capability_id))
        .filter(|capability| known_capability(capability))
        .filter(|capability| {
            registered
                .operation_for_capability(capability)
                .and_then(|operation| registered.operation_policy(operation).map(|_| operation))
                .is_ok()
        })
        .collect::<Vec<_>>();
    capabilities.sort_unstable();
    capabilities.dedup();
    capabilities
}

impl PersistenceEngine {
    pub(crate) fn validate_planned_connector_authority(
        &self,
        connector_ref: &str,
        expected_manifest: Option<&str>,
        expected_account_label: Option<&str>,
        project_id: Option<&str>,
        capability: &str,
    ) -> Result<(), String> {
        require_planned_connector_authority(
            self,
            connector_ref,
            expected_manifest,
            expected_account_label,
            project_id,
            capability,
        )
        .map(|_| ())
    }
}

pub(super) fn require_planned_connector_authority(
    engine: &PersistenceEngine,
    connector_ref: &str,
    expected_manifest: Option<&str>,
    expected_account_label: Option<&str>,
    project_id: Option<&str>,
    capability: &str,
) -> Result<ValidatedConnectorAuthority, String> {
    let account = repository::account_authority(engine, connector_ref)?
        .ok_or_else(|| "connector_planned_account_not_found".to_string())?;
    if expected_manifest.is_some_and(|expected| account.manifest_id != expected.trim()) {
        return Err("connector_planned_manifest_mismatch".to_string());
    }
    if expected_account_label.is_some_and(|expected| {
        !account
            .account_label
            .trim()
            .eq_ignore_ascii_case(expected.trim())
    }) {
        return Err("connector_planned_account_mismatch".to_string());
    }
    let registered = adapter::for_manifest(&account.manifest_id)
        .ok_or_else(|| "connector_planned_adapter_unavailable".to_string())?;
    let project_id =
        project_id.ok_or_else(|| "connector_planned_project_context_required".to_string())?;
    repository::require_project_scope(engine, connector_ref, project_id).map_err(
        |code| match code.as_str() {
            "connector_account_not_found" => "connector_planned_account_not_found".to_string(),
            "connector_account_reconnect_required" => {
                "connector_planned_account_reconnect_required".to_string()
            }
            "connector_project_context_invalid" => "connector_planned_project_invalid".to_string(),
            "connector_project_authorization_required" => {
                "connector_planned_project_authorization_required".to_string()
            }
            _ => code,
        },
    )?;
    let executable = executable_capabilities(&account.capability_grants, registered);
    let capability = capability.trim();
    if !known_capability(capability) {
        return Err("connector_planned_capability_unsupported".to_string());
    }
    let operation = registered
        .operation_for_capability(capability)
        .and_then(|operation| registered.operation_policy(operation).map(|_| operation))
        .map_err(|_| "connector_planned_capability_unsupported".to_string())?;
    if !executable.contains(&capability) {
        return Err("connector_planned_capability_consent_required".to_string());
    }
    Ok(ValidatedConnectorAuthority { operation })
}
