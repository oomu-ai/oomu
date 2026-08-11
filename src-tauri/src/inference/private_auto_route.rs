use super::{
    dynamic_routing::{self, private_apple_read::PrivateAppleReadKind},
    is_dynamic_route_id, workspace_data_resources_for_attachments, AgentManager, ChatAttachment,
    DynamicModelRouteDecision, GemmaService, InferenceError, WorkspaceDataResource,
};

pub(super) fn validate_derived_route_request(
    requested_provider_id: Option<&str>,
    requested_model_id: Option<&str>,
    parent_provider_id: &str,
    parent_model_id: &str,
) -> Result<(), InferenceError> {
    if derived_route_request_is_compatible(
        requested_provider_id,
        requested_model_id,
        parent_provider_id,
        parent_model_id,
    ) {
        return Ok(());
    }
    eprintln!(
        "DERIVED_ROUTE_MISMATCH requested_provider={} requested_model={} parent_provider={} parent_model={}",
        requested_provider_id.unwrap_or("none"),
        requested_model_id.unwrap_or("none"),
        parent_provider_id,
        parent_model_id,
    );
    Err(InferenceError::invalid(
        "Derived chat turn cannot change its parent provider or model route.",
    ))
}

pub(super) fn derived_route_request_is_compatible(
    requested_provider_id: Option<&str>,
    requested_model_id: Option<&str>,
    parent_provider_id: &str,
    parent_model_id: &str,
) -> bool {
    requested_provider_id.is_none_or(|provider_id| {
        is_dynamic_route_id(provider_id) || provider_id == parent_provider_id
    }) && requested_model_id
        .is_none_or(|model_id| is_dynamic_route_id(model_id) || model_id == parent_model_id)
}

pub(super) fn detect(
    objective: &str,
    attachments: &[ChatAttachment],
) -> Option<PrivateAppleReadKind> {
    if let Some(kind) = dynamic_routing::private_apple_read::detect_from_objective(objective) {
        return Some(kind);
    }
    if !dynamic_routing::private_apple_read::is_bounded_read_objective(objective) {
        return None;
    }
    let mut kinds = workspace_data_resources_for_attachments(attachments)
        .into_iter()
        .filter_map(resource_kind);
    let first = kinds.next()?;
    kinds.next().is_none().then_some(first)
}

pub(super) fn prepare_routing_input(
    objective: &str,
    attachments: &[ChatAttachment],
    private_read: Option<PrivateAppleReadKind>,
) -> String {
    let mut input = objective.trim().to_string();
    if private_read.is_none() && has_verified_native_public_grounding(attachments) {
        input.push_str("\n\nVerified-Native-Public-Grounding: true");
    }
    input
}

pub(super) fn route(
    message: &str,
    original_objective: Option<&str>,
    steering_only: bool,
) -> String {
    if steering_only {
        message
    } else {
        original_objective.unwrap_or(message)
    }
    .to_string()
}

fn has_verified_native_public_grounding(attachments: &[ChatAttachment]) -> bool {
    !super::public_grounding_provenance::from_attachments(attachments).is_empty()
        && attachments.iter().any(|attachment| {
            let Some(text) = attachment.text.as_deref() else {
                return false;
            };
            let receipt = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("Native-Receipt:").map(str::trim));
            receipt.is_some_and(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            }) && text
                .lines()
                .any(|line| line.trim().starts_with("Invocation-Index:"))
                && text
                    .lines()
                    .any(|line| line.trim().starts_with("Result-Count:"))
        })
}

pub(super) fn cloud_snapshot_for_turn(
    agent_manager: &AgentManager,
    private_read: Option<PrivateAppleReadKind>,
) -> Result<Option<dynamic_routing::ConfiguredCloudRouteSnapshot>, InferenceError> {
    match private_read {
        Some(_) => Ok(None),
        None => dynamic_routing::configured_cloud_route_snapshot(agent_manager),
    }
}

pub(super) async fn resolve_unforced(
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    prompt: &str,
    local_provider_id: &str,
    local_model_id: &str,
    frozen_policy_exists: bool,
    frozen_cloud: Option<&dynamic_routing::ConfiguredCloudRouteSnapshot>,
    private_read: Option<PrivateAppleReadKind>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    match (frozen_policy_exists, private_read) {
        (true, Some(kind)) => {
            dynamic_routing::private_apple_read::resolve_frozen(
                gemma,
                local_provider_id,
                local_model_id,
                kind,
            )
            .await
        }
        (true, None) => {
            dynamic_routing::resolve_dynamic_model_route_with_frozen_cloud(
                gemma,
                prompt,
                local_provider_id,
                local_model_id,
                frozen_cloud,
            )
            .await
        }
        (false, Some(kind)) => {
            dynamic_routing::private_apple_read::resolve(
                gemma,
                local_provider_id,
                local_model_id,
                kind,
            )
            .await
        }
        (false, None) => {
            dynamic_routing::resolve_dynamic_model_route(
                agent_manager,
                gemma,
                prompt,
                local_provider_id,
                local_model_id,
            )
            .await
        }
    }
}

pub(super) async fn resolve(
    agent_manager: &AgentManager,
    gemma: &GemmaService,
    prompt: &str,
    local_provider_id: &str,
    local_model_id: &str,
    choice: Option<&str>,
    cloud_confirmed: bool,
    frozen_policy_exists: bool,
    frozen_cloud: Option<&dynamic_routing::ConfiguredCloudRouteSnapshot>,
    private_read: Option<PrivateAppleReadKind>,
) -> Result<DynamicModelRouteDecision, InferenceError> {
    match (choice, frozen_policy_exists) {
        (Some(choice), true) => {
            dynamic_routing::resolve_explicit_dynamic_model_route_with_frozen_cloud(
                gemma,
                local_provider_id,
                local_model_id,
                choice,
                cloud_confirmed,
                frozen_cloud,
            )
        }
        (Some(choice), false) => dynamic_routing::resolve_explicit_dynamic_model_route(
            agent_manager,
            gemma,
            local_provider_id,
            local_model_id,
            choice,
            cloud_confirmed,
        ),
        (None, frozen_policy_exists) => {
            resolve_unforced(
                agent_manager,
                gemma,
                prompt,
                local_provider_id,
                local_model_id,
                frozen_policy_exists,
                frozen_cloud,
                private_read,
            )
            .await
        }
    }
}

fn resource_kind(resource: WorkspaceDataResource) -> Option<PrivateAppleReadKind> {
    match resource {
        WorkspaceDataResource::Mail => Some(PrivateAppleReadKind::Mail),
        WorkspaceDataResource::Calendar => Some(PrivateAppleReadKind::Calendar),
        WorkspaceDataResource::Reminders => Some(PrivateAppleReadKind::Reminders),
        WorkspaceDataResource::Notes => Some(PrivateAppleReadKind::Notes),
        WorkspaceDataResource::Contacts => Some(PrivateAppleReadKind::Contacts),
        WorkspaceDataResource::Photos => Some(PrivateAppleReadKind::Photos),
        WorkspaceDataResource::Music => Some(PrivateAppleReadKind::Music),
        WorkspaceDataResource::AppleAppUi => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprint_304_routing_input_marks_only_receipt_backed_native_grounding() {
        let text = concat!(
            "Local Web Search Context\n",
            "Native-Receipt: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
            "Invocation-Index: 1\n",
            "Result-Count: 1\n\n",
            "{\"accessedAtUtc\":\"2026-07-31T12:00:00Z\",",
            "\"pages\":[{\"url\":\"https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac\"}]}"
        );
        let attachment = ChatAttachment {
            name: "local_web_search.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: text.len(),
            data_base64: None,
            text: Some(text.to_string()),
            approved_file_receipt: None,
        };

        assert!(
            prepare_routing_input("Look online for Spotlight.", &[attachment], None)
                .contains("Verified-Native-Public-Grounding: true")
        );
        assert!(!prepare_routing_input("I will look online.", &[], None)
            .contains("Verified-Native-Public-Grounding: true"));
    }

    #[test]
    fn auto_route_does_not_mix_approved_file_evidence_into_the_objective() {
        let attachment_text = concat!(
            "Approved local file receipt\n",
            "Path: [approved file]\n",
            "Content: routine inventory rows that must not influence routing"
        );
        let attachment = ChatAttachment {
            name: "Lab_Inventory.csv".to_string(),
            mime_type: "text/csv".to_string(),
            byte_count: attachment_text.len(),
            data_base64: None,
            text: Some(attachment_text.to_string()),
            approved_file_receipt: None,
        };
        let objective = "Compare two supplier files and produce a multi-scenario trade-off matrix.";

        let routing_input = prepare_routing_input(objective, &[attachment], None);

        assert_eq!(routing_input, objective);
        assert!(!routing_input.contains("Approved local file receipt"));
        assert!(!routing_input.contains("[approved file]"));
    }

    #[test]
    fn auto_route_classifies_the_original_objective_not_the_sanitized_execution_prompt() {
        let original = concat!(
            "Perform a comprehensive strategic evaluation of supplier_proposals.json and ",
            "q3_strategic_vendor_proposals.txt. Compare technical compliance, unit pricing, ",
            "and delivery risks, and provide a multi-scenario vendor trade-off matrix."
        );
        let runtime = concat!(
            "Perform a comprehensive strategic evaluation of [approved file] and ",
            "[approved file]."
        );

        let classifier_input = route(runtime, Some(original), false);

        assert_eq!(classifier_input, original);
        assert!(!classifier_input.contains("[approved file]"));
    }

    #[test]
    fn steering_classification_keeps_the_active_runtime_objective() {
        let classifier_input = route(
            "Use the cloud model for this follow-up.",
            Some("Original parent objective."),
            true,
        );

        assert_eq!(classifier_input, "Use the cloud model for this follow-up.");
    }
}
