use super::*;

#[derive(Debug, Clone, Deserialize)]
pub struct AcceptChatTurnRequest {
    pub turn_id: String,
    pub generation_token: String,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
    pub root_turn_id: String,
    pub turn_kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedChatTurn {
    pub turn_id: String,
    pub message_id: i64,
    pub accepted: bool,
    pub session_was_empty_before_acceptance: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FinalizeAcceptedChatTurnRequest {
    pub turn_id: String,
    pub generation_token: String,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
    pub root_turn_id: String,
    pub turn_kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub role: String,
    pub content: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AbandonAcceptedChatTurnRequest {
    pub turn_id: String,
    pub generation_token: String,
    #[serde(default)]
    pub parent_turn_id: Option<String>,
    pub root_turn_id: String,
    pub turn_kind: String,
    pub session_id: String,
    pub agent_id: String,
    pub provider_id: String,
    pub model_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelSavedChatTurnRequest {
    pub session_id: String,
    pub turn_id: String,
    pub generation_token: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CompleteClaimedChatTurnRequest {
    pub context: ChatTurnPersistenceContext,
    pub role: String,
    pub content: String,
    pub message_provider_id: String,
    pub message_model_id: String,
    pub metadata: Value,
    pub session_title: Option<String>,
    pub session_provider_id: String,
    pub session_model_id: String,
    pub status: String,
}

impl AcceptChatTurnRequest {
    pub(crate) fn persistence_context(&self) -> ChatTurnPersistenceContext {
        persistence_context(
            &self.turn_id,
            &self.generation_token,
            self.parent_turn_id.as_deref(),
            &self.root_turn_id,
            &self.turn_kind,
            &self.session_id,
            &self.agent_id,
            &self.provider_id,
            &self.model_id,
        )
    }
}

impl FinalizeAcceptedChatTurnRequest {
    pub(super) fn persistence_context(&self) -> ChatTurnPersistenceContext {
        persistence_context(
            &self.turn_id,
            &self.generation_token,
            self.parent_turn_id.as_deref(),
            &self.root_turn_id,
            &self.turn_kind,
            &self.session_id,
            &self.agent_id,
            &self.provider_id,
            &self.model_id,
        )
    }
}

impl AbandonAcceptedChatTurnRequest {
    pub(super) fn persistence_context(&self) -> ChatTurnPersistenceContext {
        persistence_context(
            &self.turn_id,
            &self.generation_token,
            self.parent_turn_id.as_deref(),
            &self.root_turn_id,
            &self.turn_kind,
            &self.session_id,
            &self.agent_id,
            &self.provider_id,
            &self.model_id,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn persistence_context(
    turn_id: &str,
    generation_token: &str,
    parent_turn_id: Option<&str>,
    root_turn_id: &str,
    turn_kind: &str,
    session_id: &str,
    agent_id: &str,
    provider_id: &str,
    model_id: &str,
) -> ChatTurnPersistenceContext {
    ChatTurnPersistenceContext {
        turn_id: turn_id.trim().to_string(),
        generation_token: generation_token.trim().to_string(),
        session_id: session_id.trim().to_string(),
        agent_id: agent_id.trim().to_string(),
        provider_id: provider_id.trim().to_string(),
        model_id: model_id.trim().to_string(),
        parent_turn_id: parent_turn_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        root_turn_id: root_turn_id.trim().to_string(),
        turn_kind: turn_kind.trim().to_string(),
    }
}
