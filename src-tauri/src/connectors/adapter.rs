use super::{auth::ConnectorCredential, ConnectorCapabilityGrant};
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct OperationPolicy {
    pub origin: &'static str,
    pub citation: &'static str,
    pub remote: bool,
    pub effectful: bool,
    pub data_classes: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct AdapterExecution {
    pub result: Value,
    pub partial: bool,
    pub freshness: &'static str,
    pub citation: String,
}

/// Executable connector seam consumed by the generic Project/Shield runtime.
/// A new service implements this trait and registers here; Tasks and Chat stay
/// independent of provider-specific operations.
pub(super) trait ConnectorAdapter: Send + Sync {
    fn operation_for_capability(&self, capability: &str) -> Result<&'static str, String>;

    fn capabilities_for_operation(&self, operation: &str) -> Vec<&'static str>;

    fn operation_policy(&self, operation: &str) -> Result<OperationPolicy, String>;

    fn execute(
        &self,
        credential: Option<&ConnectorCredential>,
        operation: &str,
        arguments: &Value,
    ) -> Result<AdapterExecution, String>;

    fn approval_arguments(&self, operation: &str, arguments: &Value) -> Result<Value, String> {
        let _ = operation;
        Ok(arguments.clone())
    }

    fn capability_grants(
        &self,
        granted_scopes: &[String],
        account_kind: Option<&str>,
    ) -> Vec<ConnectorCapabilityGrant>;
}

pub(super) fn for_manifest(manifest_id: &str) -> Option<&'static dyn ConnectorAdapter> {
    match manifest_id {
        super::microsoft365::MANIFEST_ID => Some(&super::microsoft365::MICROSOFT_ADAPTER),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microsoft_is_available_through_the_generic_adapter_registry() {
        let adapter = for_manifest(super::super::microsoft365::MANIFEST_ID).unwrap();
        let policy = adapter
            .operation_policy(super::super::microsoft365::OUTLOOK_MAIL_SEARCH)
            .unwrap();
        assert!(policy.remote);
        assert!(!policy.effectful);
        assert_eq!(policy.origin, "https://graph.microsoft.com");
    }
}
