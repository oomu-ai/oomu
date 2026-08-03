use super::{objective_policy, SEMANTIC_CLASSIFIER_VERSION};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SemanticDemand {
    Routine,
    Advanced,
}

impl SemanticDemand {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Routine => "routine",
            Self::Advanced => "advanced",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum SemanticCapability {
    #[serde(rename = "g")]
    General,
    #[serde(rename = "m")]
    MathematicalReasoning,
    #[serde(rename = "l")]
    LegalCompliance,
    #[serde(rename = "a")]
    SystemArchitecture,
    #[serde(rename = "r")]
    ResearchSynthesis,
    #[serde(rename = "c")]
    CodeAnalysis,
    #[serde(rename = "x")]
    MultiConstraintReasoning,
    #[serde(rename = "s")]
    SpecialistJudgment,
    #[serde(rename = "u")]
    Uncertain,
}

impl SemanticCapability {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::MathematicalReasoning => "mathematical_reasoning",
            Self::LegalCompliance => "legal_compliance",
            Self::SystemArchitecture => "system_architecture",
            Self::ResearchSynthesis => "research_synthesis",
            Self::CodeAnalysis => "code_analysis",
            Self::MultiConstraintReasoning => "multi_constraint_reasoning",
            Self::SpecialistJudgment => "specialist_judgment",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SemanticClassifierCode {
    Classified {
        demand: SemanticDemand,
        capability: SemanticCapability,
    },
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SemanticConfidence {
    Confident,
    Low,
}

impl SemanticConfidence {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Confident => "confident",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SemanticReason {
    BoundedTransformation,
    SpecialistReasoning,
    SourceSynthesis,
    CrossConstraintAnalysis,
    HighStakesJudgment,
    Uncertain,
    ExplicitUserChoice,
}

impl SemanticReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::BoundedTransformation => "bounded_transformation",
            Self::SpecialistReasoning => "specialist_reasoning",
            Self::SourceSynthesis => "source_synthesis",
            Self::CrossConstraintAnalysis => "cross_constraint_analysis",
            Self::HighStakesJudgment => "high_stakes_judgment",
            Self::Uncertain => "uncertain",
            Self::ExplicitUserChoice => "explicit_user_choice",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct SemanticAssessment {
    pub(super) demand: SemanticDemand,
    pub(super) capability: SemanticCapability,
    pub(super) confidence: SemanticConfidence,
    pub(super) reason: SemanticReason,
    pub(super) source: String,
    pub(super) classifier_latency_ms: u128,
    pub(super) classifier_model_id: Option<String>,
    pub(super) readiness_generation: u64,
    pub(super) recovery_attempted: bool,
}

impl SemanticAssessment {
    pub(super) fn from_code(code: SemanticClassifierCode) -> Self {
        let (demand, capability, confidence) = match code {
            SemanticClassifierCode::Classified { demand, capability } => {
                (demand, capability, SemanticConfidence::Confident)
            }
            SemanticClassifierCode::Uncertain => (
                SemanticDemand::Advanced,
                SemanticCapability::Uncertain,
                SemanticConfidence::Low,
            ),
        };
        let reason = match (demand, capability) {
            (_, SemanticCapability::Uncertain) => SemanticReason::Uncertain,
            (SemanticDemand::Routine, _) => SemanticReason::BoundedTransformation,
            (_, SemanticCapability::LegalCompliance | SemanticCapability::SpecialistJudgment) => {
                SemanticReason::HighStakesJudgment
            }
            (_, SemanticCapability::ResearchSynthesis) => SemanticReason::SourceSynthesis,
            (_, SemanticCapability::MultiConstraintReasoning) => {
                SemanticReason::CrossConstraintAnalysis
            }
            _ => SemanticReason::SpecialistReasoning,
        };
        Self {
            demand,
            capability,
            confidence,
            reason,
            source: SEMANTIC_CLASSIFIER_VERSION.to_string(),
            classifier_latency_ms: 0,
            classifier_model_id: None,
            readiness_generation: 0,
            recovery_attempted: false,
        }
    }

    pub(super) fn requires_cloud(&self) -> bool {
        self.demand == SemanticDemand::Advanced
    }

    pub(super) fn audit_signals(&self) -> Vec<String> {
        vec![
            format!("semantic:capability={}", self.capability.as_str()),
            format!("semantic:demand={}", self.demand.as_str()),
            format!("semantic:confidence={}", self.confidence.as_str()),
            format!("semantic:reason={}", self.reason.as_str()),
            format!("semantic:source={}", self.source),
            format!(
                "semantic:readiness_generation={}",
                self.readiness_generation
            ),
        ]
    }

    pub(super) fn cloud_basis(&self) -> String {
        if self.source == objective_policy::CURRENT_RESEARCH_POLICY_VERSION {
            return "OOMU's deterministic current-research policy identified a source-bound, multi-stage public research request"
                .to_string();
        }
        if self
            .source
            .contains(objective_policy::OBJECTIVE_SEMANTIC_FLOOR_VERSION)
        {
            return format!(
                "The ready local classifier and OOMU's bounded semantic policy identified advanced {} work",
                self.capability.as_str(),
            );
        }
        if self.capability == SemanticCapability::Uncertain {
            "The ready local difficulty classifier returned a grammar-valid uncertain decision"
                .to_string()
        } else {
            format!(
                "The local difficulty classifier returned a validated advanced {} classification",
                self.capability.as_str(),
            )
        }
    }
}
