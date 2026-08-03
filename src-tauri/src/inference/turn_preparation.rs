use super::{
    clean_runtime_text, finish_prebound_chat_turn, hydrate_approved_file_receipts,
    validate_chat_attachments, ChatAttachment, InferenceError,
};
use crate::{
    agent_manager::AgentManager,
    db::{ChatTurnPersistenceContext, PersistenceEngine},
    sovereign_identity::SovereignIdentity,
};

pub(super) struct PreparedTurnAttachments {
    pub(super) attachments: Vec<ChatAttachment>,
    pub(super) display_message: Option<String>,
    pub(super) has_verified_approved_file_context: bool,
}

pub(super) fn prepare_turn_attachments(
    mut attachments: Vec<ChatAttachment>,
    requested_display_message: Option<String>,
    message: &str,
    identity: &SovereignIdentity,
    session_id: Option<&str>,
    root_turn_id: &str,
    agent_id: &str,
    persistence: &PersistenceEngine,
    turn_id: &str,
    generation_token: &str,
) -> Result<PreparedTurnAttachments, InferenceError> {
    let hydration = match hydrate_approved_file_receipts(
        &mut attachments,
        identity,
        session_id.unwrap_or_default(),
        root_turn_id,
        agent_id,
    ) {
        Ok(hydration) => hydration,
        Err(reason) => {
            eprintln!(
                "APPROVED_FILE_RECEIPT_REJECTED turn={} attachment_count={} reason={}",
                crate::foundation::digest::sha256_hex(turn_id.as_bytes()),
                attachments.len(),
                reason
            );
            finish_prebound_chat_turn(persistence, turn_id, generation_token);
            return Err(InferenceError::approved_file_unavailable());
        }
    };
    let has_verified_approved_file_context = hydration.verified_receipt_count > 0;
    let display_message =
        clean_runtime_text(requested_display_message).or(hydration.display_message);
    if let Err(reason) = validate_chat_attachments(&attachments) {
        if has_verified_approved_file_context {
            finish_prebound_chat_turn(persistence, turn_id, generation_token);
        }
        return Err(InferenceError::invalid(reason));
    }
    if crate::agentic_loop::contains_approved_file_marker(message)
        && !has_verified_approved_file_context
    {
        eprintln!(
            "APPROVED_FILE_CONTEXT_MISSING turn={} attachment_count={}",
            crate::foundation::digest::sha256_hex(turn_id.as_bytes()),
            attachments.len()
        );
        finish_prebound_chat_turn(persistence, turn_id, generation_token);
        return Err(InferenceError::approved_file_unavailable());
    }
    if message.is_empty() && attachments.is_empty() {
        return Err(InferenceError::invalid("Chat message cannot be empty."));
    }
    Ok(PreparedTurnAttachments {
        attachments,
        display_message,
        has_verified_approved_file_context,
    })
}

pub(super) fn load_parent_turn_context(
    persistence: &PersistenceEngine,
    parent_turn_id: Option<&str>,
    agent_id: &str,
    session_id: Option<&str>,
) -> Result<Option<ChatTurnPersistenceContext>, InferenceError> {
    let parent = parent_turn_id
        .map(|parent_turn_id| {
            persistence
                .select_chat_turn_context(parent_turn_id)
                .map_err(|error| InferenceError::worker(error.to_string()))?
                .ok_or_else(|| InferenceError::invalid("Derived chat turn parent was not found."))
        })
        .transpose()?;
    if parent.as_ref().is_some_and(|parent| {
        parent.agent_id != agent_id || session_id.map(str::trim) != Some(parent.session_id.as_str())
    }) {
        return Err(InferenceError::invalid(
            "Derived chat turn cannot cross its parent session or agent boundary.",
        ));
    }
    Ok(parent)
}

pub(super) fn verified_native_execution_authority(
    receipt_id: Option<&str>,
    parent: Option<&ChatTurnPersistenceContext>,
    steering_only: bool,
    has_verified_approved_file_context: bool,
    turn_kind: &str,
    legacy_receipt_claim: bool,
) -> Result<bool, InferenceError> {
    if receipt_id.is_some() && (!steering_only || parent.is_none()) {
        return Err(InferenceError::invalid(
            "A native execution receipt can only continue its bound parent turn.",
        ));
    }
    let consumed_receipt = receipt_id
        .zip(parent)
        .map(|(receipt_id, parent)| {
            crate::tools::native_operation_receipt::consume_chat_turn_receipt(receipt_id, parent)
                .map_err(|error| {
                    InferenceError::invalid(format!(
                        "The native execution receipt could not be verified. ({})",
                        error.code()
                    ))
                })
        })
        .transpose()?;
    Ok(has_verified_approved_file_context
        || consumed_receipt
            .as_ref()
            .is_some_and(|receipt| receipt.verified_success)
        || (turn_kind == crate::db::AUTO_TURN_KIND && legacy_receipt_claim))
}

pub(super) async fn resolve_bound_mod_ids(
    agent_manager: &AgentManager,
    agent_id: &str,
    safe_mode: bool,
    requested_mod_id: Option<&str>,
) -> Result<Vec<String>, InferenceError> {
    let mut bound_mod_ids = agent_manager
        .get_agent_mods(agent_id.to_string())
        .await
        .map_err(InferenceError::worker)?;
    if safe_mode {
        bound_mod_ids.clear();
    }
    if let Some(requested_mod_id) = requested_mod_id {
        if !bound_mod_ids
            .iter()
            .any(|mod_id| mod_id == requested_mod_id)
        {
            bound_mod_ids.push(requested_mod_id.to_string());
        }
    }
    Ok(bound_mod_ids)
}
