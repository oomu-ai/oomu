use super::ChatSessionRecord;
use serde::{Deserialize, Serialize};

macro_rules! text_identity {
    ($name:ident, $label:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let value = value.trim();
                if value.is_empty() {
                    return Err(concat!($label, " cannot be empty.").to_string());
                }
                Ok(Self(value.to_string()))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

text_identity!(ProviderConfigurationId, "Provider configuration ID");
text_identity!(ProviderTypeId, "Provider type");
text_identity!(CanonicalModelId, "Canonical model ID");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RouteGeneration(i64);

impl RouteGeneration {
    pub const UNVERIFIED: Self = Self(0);

    pub fn verified(value: i64) -> Result<Self, String> {
        if value <= 0 {
            return Err("Route generation must be positive.".to_string());
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub(crate) fn from_persisted(value: i64) -> Self {
        Self(value.max(0))
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1).max(1))
    }
}

impl Default for RouteGeneration {
    fn default() -> Self {
        Self::UNVERIFIED
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoRouteProvenance {
    LegacyUnverified,
    ExplicitSession,
    AgentAssignment,
    StartupDefault,
    VerifiedLegacyRepair,
    NeedsUserChoice,
}

impl AutoRouteProvenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacyUnverified => "legacy_unverified",
            Self::ExplicitSession => "explicit_session",
            Self::AgentAssignment => "agent_assignment",
            Self::StartupDefault => "startup_default",
            Self::VerifiedLegacyRepair => "verified_legacy_repair",
            Self::NeedsUserChoice => "needs_user_choice",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteSessionBaselineRequest {
    pub provider_config_id: ProviderConfigurationId,
    pub provider_type: ProviderTypeId,
    pub model_id: CanonicalModelId,
    pub reasoning_depth: String,
    pub context_budget: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedAutoRouteBaseline {
    pub provider_config_id: ProviderConfigurationId,
    pub provider_type: ProviderTypeId,
    pub model_id: CanonicalModelId,
    pub reasoning_depth: String,
    pub context_budget: i32,
    pub provenance: AutoRouteProvenance,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteActivationReceipt {
    pub kind: &'static str,
    pub receipt_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_config_id: Option<ProviderConfigurationId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<ProviderTypeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<CanonicalModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<AutoRouteProvenance>,
    pub previous_route_generation: RouteGeneration,
    pub current_route_generation: RouteGeneration,
    pub previous_state_digest: String,
    pub current_state_digest: String,
    pub dynamic_routing_enabled: bool,
    pub changed: bool,
    pub committed: bool,
    pub rolled_back: bool,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoRouteActivationResponse {
    pub session: ChatSessionRecord,
    pub receipt: AutoRouteActivationReceipt,
}
