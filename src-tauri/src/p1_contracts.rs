use crate::p0_contracts::{ArtifactId, ChildRunId, EvidenceClass, ProjectId, TaskId, TaskRunId};
use chrono::DateTime;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fmt;

pub const P1_CONTRACT_VERSION: u16 = 1;

fn valid_uuid_suffix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 8 | 13 | 18 | 23)
                || byte.is_ascii_digit()
                || (b'a'..=b'f').contains(byte)
        })
        && (b'1'..=b'8').contains(&bytes[14])
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
        && bytes
            .iter()
            .any(|byte| byte.is_ascii_hexdigit() && *byte != b'0')
}

macro_rules! p1_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, String> {
                let value = value.into();
                let suffix = value
                    .strip_prefix(concat!($prefix, "_"))
                    .ok_or_else(|| concat!("invalid ", $prefix, " identifier").to_string())?;
                if !valid_uuid_suffix(suffix) {
                    return Err(concat!("invalid ", $prefix, " identifier").to_string());
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

p1_id!(ObservationId, "observation");
p1_id!(DesktopActionId, "action");
p1_id!(MediaAssetId, "media");
p1_id!(RemoteDeviceId, "device");
p1_id!(CapabilityBundleId, "bundle");
p1_id!(LearningCandidateId, "learning");
p1_id!(WorkGraphId, "workgraph");

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum P1ContractType {
    ArtifactWorkbook,
    ArtifactPresentation,
    DesktopObservation,
    DesktopAction,
    MediaAsset,
    RemoteDevice,
    CapabilityBundle,
    LearningCandidate,
    WorkGraph,
}

fn validate_header(
    schema_version: u16,
    actual: P1ContractType,
    expected: P1ContractType,
) -> Result<(), String> {
    if schema_version != P1_CONTRACT_VERSION {
        return Err("unsupported P1 contract version".to_string());
    }
    if actual != expected {
        return Err("P1 contract type does not match its schema".to_string());
    }
    Ok(())
}

fn validate_string(value: &str, min: usize, max: usize, label: &str) -> Result<(), String> {
    let value = value.trim();
    if value.len() < min || value.len() > max {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    if !value.ends_with('Z') || DateTime::parse_from_rfc3339(value).is_err() {
        return Err("timestamp must be a valid UTC RFC 3339 value".to_string());
    }
    Ok(())
}

fn validate_lower_hex(value: &str, bytes: usize, label: &str) -> Result<(), String> {
    if value.len() != bytes * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("invalid {label}"));
    }
    Ok(())
}

fn validate_evidence_project(
    project_id: &ProjectId,
    evidence: &[P1EvidenceReference],
) -> Result<(), String> {
    if evidence.len() > 4_096 {
        return Err("too many evidence references".to_string());
    }
    if evidence.iter().any(|item| &item.project_id != project_id) {
        return Err("cross-project evidence reference".to_string());
    }
    for item in evidence {
        item.validate()?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct P1EvidenceReference {
    pub project_id: ProjectId,
    pub evidence_class: EvidenceClass,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_run_id: Option<TaskRunId>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl P1EvidenceReference {
    fn validate(&self) -> Result<(), String> {
        validate_string(&self.reference, 1, 1_024, "evidence reference")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SignatureAlgorithm {
    Ed25519,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedEnvelope {
    pub algorithm: SignatureAlgorithm,
    pub key_id: String,
    pub payload_sha256: String,
    pub signature: String,
    pub signed_at: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl SignedEnvelope {
    fn validate(&self) -> Result<(), String> {
        validate_string(&self.key_id, 1, 512, "signature key identifier")?;
        validate_lower_hex(&self.payload_sha256, 32, "signed payload digest")?;
        validate_lower_hex(&self.signature, 64, "Ed25519 signature")?;
        validate_timestamp(&self.signed_at)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBudgetLimits {
    pub tokens: u64,
    pub wall_time_ms: u64,
    pub memory_bytes: u64,
    pub processes: u64,
    pub network_requests: u64,
    pub tool_calls: u64,
    pub concurrent_children: u8,
    pub mutations: u64,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBudgetUsage {
    pub tokens: u64,
    pub wall_time_ms: u64,
    pub peak_memory_bytes: u64,
    pub processes: u64,
    pub network_requests: u64,
    pub tool_calls: u64,
    pub peak_concurrent_children: u8,
    pub mutation_attempts: u64,
    pub mutations_committed: u64,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBudgetTelemetry {
    pub limits: ResourceBudgetLimits,
    pub usage: ResourceBudgetUsage,
    pub sampled_at: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

impl ResourceBudgetTelemetry {
    fn validate(&self) -> Result<(), String> {
        if !(1..=8).contains(&self.limits.concurrent_children)
            || self.usage.peak_concurrent_children > 8
            || self.usage.peak_concurrent_children > self.limits.concurrent_children
            || self.usage.tokens > self.limits.tokens
            || self.usage.wall_time_ms > self.limits.wall_time_ms
            || self.usage.peak_memory_bytes > self.limits.memory_bytes
            || self.usage.processes > self.limits.processes
            || self.usage.network_requests > self.limits.network_requests
            || self.usage.tool_calls > self.limits.tool_calls
            || self.usage.mutation_attempts > self.limits.mutations
            || self.usage.mutations_committed > self.usage.mutation_attempts
        {
            return Err("resource usage exceeds its declared budget".to_string());
        }
        validate_timestamp(&self.sampled_at)
    }
}

macro_rules! validated_contract {
    (
        $name:ident, $raw:ident {
            $( $(#[$meta:meta])* $field:ident : $field_type:ty ),* $(,)?
        }
        validate |$value:ident| $body:block
    ) => {
        #[derive(Clone, Debug, PartialEq, Serialize)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            $( $(#[$meta])* pub $field: $field_type, )*
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct $raw {
            $( $(#[$meta])* $field: $field_type, )*
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let raw = $raw::deserialize(deserializer)?;
                let result = Self { $( $field: raw.$field, )* };
                let $value = &result;
                let validation: Result<(), String> = (|| $body)();
                validation.map_err(D::Error::custom)?;
                Ok(result)
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum WorkbookDateSystem {
    #[serde(rename = "1900")]
    Excel1900,
    #[serde(rename = "1904")]
    Excel1904,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbookWorksheet {
    pub sheet_id: String,
    pub project_id: ProjectId,
    pub name: String,
    pub row_count: u64,
    pub column_count: u64,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

validated_contract!(
    ArtifactWorkbook, RawArtifactWorkbook {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        task_id: TaskId,
        task_run_id: TaskRunId,
        artifact_id: ArtifactId,
        revision: u64,
        locale: String,
        date_system: WorkbookDateSystem,
        worksheets: Vec<WorkbookWorksheet>,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::ArtifactWorkbook)?;
        if value.revision == 0 || value.worksheets.is_empty() || value.worksheets.len() > 1_024 {
            return Err("invalid workbook revision or worksheet count".to_string());
        }
        validate_string(&value.locale, 2, 35, "workbook locale")?;
        for sheet in &value.worksheets {
            if sheet.project_id != value.project_id {
                return Err("cross-project worksheet reference".to_string());
            }
            validate_string(&sheet.sheet_id, 1, 512, "worksheet identifier")?;
            validate_string(&sheet.name, 1, 128, "worksheet name")?;
        }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PresentationAspectRatio {
    #[serde(rename = "16:9")]
    Widescreen,
    #[serde(rename = "4:3")]
    Standard,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSlide {
    pub slide_id: String,
    pub project_id: ProjectId,
    pub position: u64,
    pub layout_id: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

validated_contract!(
    ArtifactPresentation, RawArtifactPresentation {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        task_id: TaskId,
        task_run_id: TaskRunId,
        artifact_id: ArtifactId,
        revision: u64,
        aspect_ratio: PresentationAspectRatio,
        slides: Vec<PresentationSlide>,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::ArtifactPresentation)?;
        if value.revision == 0 || value.slides.is_empty() || value.slides.len() > 1_000 {
            return Err("invalid presentation revision or slide count".to_string());
        }
        for slide in &value.slides {
            if slide.project_id != value.project_id {
                return Err("cross-project slide reference".to_string());
            }
            validate_string(&slide.slide_id, 1, 512, "slide identifier")?;
            validate_string(&slide.layout_id, 1, 512, "slide layout identifier")?;
        }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopApplication {
    pub bundle_id: String,
    pub process_id: u64,
    pub name: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopWindow {
    pub window_ref: String,
    pub title: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopElementReference {
    pub element_ref: String,
    pub project_id: ProjectId,
    pub role: String,
    pub secure: bool,
    pub expires_at: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

validated_contract!(
    DesktopObservation, RawDesktopObservation {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        task_id: TaskId,
        task_run_id: TaskRunId,
        observation_id: ObservationId,
        revision: u64,
        observed_at: String,
        application: DesktopApplication,
        window: DesktopWindow,
        elements: Vec<DesktopElementReference>,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::DesktopObservation)?;
        if value.revision == 0 || value.elements.len() > 100_000 || value.application.process_id == 0 {
            return Err("invalid desktop observation revision, process, or element count".to_string());
        }
        validate_timestamp(&value.observed_at)?;
        validate_string(&value.application.bundle_id, 3, 255, "application bundle identifier")?;
        validate_string(&value.application.name, 1, 512, "application name")?;
        validate_string(&value.window.window_ref, 1, 1_024, "window reference")?;
        if value.window.title.len() > 512 { return Err("invalid window title".to_string()); }
        for element in &value.elements {
            if element.project_id != value.project_id { return Err("cross-project desktop element reference".to_string()); }
            validate_string(&element.element_ref, 1, 1_024, "desktop element reference")?;
            validate_string(&element.role, 1, 512, "desktop element role")?;
            validate_timestamp(&element.expires_at)?;
        }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopActionKind {
    Focus,
    Press,
    Select,
    Type,
    InvokeMenu,
    Scroll,
    DragDrop,
    ChooseFile,
    AppleEvent,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopActionTarget {
    pub project_id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_ref: Option<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopPostconditionKind {
    ElementValue,
    ElementState,
    WindowState,
    FileHash,
    ApplicationState,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopExpectedPostcondition {
    pub kind: DesktopPostconditionKind,
    pub description: String,
    pub evidence_class: EvidenceClass,
    pub parameters: BTreeMap<String, Value>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

validated_contract!(
    DesktopAction, RawDesktopAction {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        task_id: TaskId,
        task_run_id: TaskRunId,
        action_id: DesktopActionId,
        observation_id: ObservationId,
        observation_revision: u64,
        action_kind: DesktopActionKind,
        application_bundle_id: String,
        target: DesktopActionTarget,
        #[serde(default, skip_serializing_if = "Option::is_none")] approval_id: Option<String>,
        requested_at: String,
        arguments: BTreeMap<String, Value>,
        expected_postcondition: DesktopExpectedPostcondition,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::DesktopAction)?;
        if value.observation_revision == 0 || value.target.project_id != value.project_id {
            return Err("invalid or cross-project desktop action target".to_string());
        }
        validate_string(&value.application_bundle_id, 3, 255, "application bundle identifier")?;
        if let Some(reference) = value.target.element_ref.as_deref() { validate_string(reference, 1, 1_024, "desktop element reference")?; }
        if let Some(approval_id) = value.approval_id.as_deref() {
            let suffix = approval_id.strip_prefix("trustgrant_").ok_or_else(|| "invalid approval identifier".to_string())?;
            if suffix.len() != 36 || !suffix.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err("invalid approval identifier".to_string());
            }
        }
        validate_timestamp(&value.requested_at)?;
        validate_string(&value.expected_postcondition.description, 1, 2_000, "postcondition description")?;
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Audio,
    Image,
    Video,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaSourceKind {
    Microphone,
    VoiceMessage,
    Screenshot,
    Clipboard,
    Camera,
    ProjectFile,
    Generated,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSource {
    pub kind: MediaSourceKind,
    pub project_id: ProjectId,
    pub reference: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaRelationshipKind {
    Source,
    Derivative,
    Transcript,
    Thumbnail,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRelationship {
    pub media_asset_id: MediaAssetId,
    pub project_id: ProjectId,
    pub relationship: MediaRelationshipKind,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionMode {
    Task,
    Project,
    Until,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRetentionPolicy {
    pub mode: RetentionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionState {
    NotRequired,
    Required,
    Applied,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRedactionPolicy {
    pub state: RedactionState,
    pub categories: Vec<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRoutingMode {
    LocalOnly,
    ApprovedProviders,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProviderRoutingPolicy {
    pub mode: ProviderRoutingMode,
    pub provider_ids: Vec<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

fn valid_mime_type(value: &str) -> bool {
    fn valid_part(value: &str) -> bool {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&byte))
    }
    let mut parts = value.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(left), Some(right), None) if valid_part(left) && valid_part(right))
}

validated_contract!(
    MediaAsset, RawMediaAsset {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        #[serde(default, skip_serializing_if = "Option::is_none")] task_id: Option<TaskId>,
        #[serde(default, skip_serializing_if = "Option::is_none")] task_run_id: Option<TaskRunId>,
        media_asset_id: MediaAssetId,
        media_kind: MediaKind,
        mime_type: String,
        sha256: String,
        byte_length: u64,
        created_at: String,
        source: MediaSource,
        retention_policy: MediaRetentionPolicy,
        redaction_policy: MediaRedactionPolicy,
        provider_routing_policy: MediaProviderRoutingPolicy,
        related_assets: Vec<MediaRelationship>,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::MediaAsset)?;
        if !valid_mime_type(&value.mime_type) { return Err("invalid media MIME type".to_string()); }
        validate_lower_hex(&value.sha256, 32, "media digest")?;
        validate_timestamp(&value.created_at)?;
        if value.source.project_id != value.project_id { return Err("cross-project media source".to_string()); }
        validate_string(&value.source.reference, 1, 1_024, "media source reference")?;
        if value.related_assets.len() > 4_096 || value.related_assets.iter().any(|item| item.project_id != value.project_id) {
            return Err("invalid or cross-project related media reference".to_string());
        }
        if value.retention_policy.mode == RetentionMode::Until && value.retention_policy.expires_at.is_none() {
            return Err("until-based retention requires an expiry".to_string());
        }
        if let Some(expires_at) = value.retention_policy.expires_at.as_deref() { validate_timestamp(expires_at)?; }
        if value.redaction_policy.categories.len() > 256 { return Err("too many redaction categories".to_string()); }
        for category in &value.redaction_policy.categories { validate_string(category, 1, 512, "redaction category")?; }
        if value.provider_routing_policy.provider_ids.len() > 256
            || (value.provider_routing_policy.mode == ProviderRoutingMode::LocalOnly && !value.provider_routing_policy.provider_ids.is_empty()) {
            return Err("invalid media provider routing policy".to_string());
        }
        for provider in &value.provider_routing_policy.provider_ids { validate_string(provider, 1, 512, "provider identifier")?; }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteDeviceScope {
    CreateTask,
    ViewTask,
    SteerTask,
    StopTask,
    AnswerClarification,
    ApproveBoundedAction,
    RequestArtifact,
}

validated_contract!(
    RemoteDevice, RawRemoteDevice {
        schema_version: u16,
        contract_type: P1ContractType,
        remote_device_id: RemoteDeviceId,
        label: String,
        public_key: String,
        allowed_project_ids: Vec<ProjectId>,
        scopes: Vec<RemoteDeviceScope>,
        paired_at: String,
        expires_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] revoked_at: Option<String>,
        evidence: Vec<P1EvidenceReference>,
        signature: SignedEnvelope,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::RemoteDevice)?;
        validate_string(&value.label, 1, 512, "remote device label")?;
        validate_lower_hex(&value.public_key, 32, "remote device public key")?;
        if value.allowed_project_ids.is_empty() || value.allowed_project_ids.len() > 1_024 || value.scopes.is_empty() {
            return Err("remote device requires a bounded Project and command scope".to_string());
        }
        let allowed = value.allowed_project_ids.iter().collect::<HashSet<_>>();
        if value.evidence.len() > 4_096 || value.evidence.iter().any(|item| !allowed.contains(&item.project_id)) {
            return Err("cross-project remote device evidence".to_string());
        }
        for item in &value.evidence { item.validate()?; }
        validate_timestamp(&value.paired_at)?;
        validate_timestamp(&value.expires_at)?;
        if let Some(revoked_at) = value.revoked_at.as_deref() { validate_timestamp(revoked_at)?; }
        value.signature.validate()
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    File,
    Network,
    Connector,
    Model,
    Executable,
    Schedule,
    ChildAgent,
    Mutation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BundlePublisher {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleScopeKind {
    Project,
    Global,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleScope {
    pub kind: BundleScopeKind,
    pub project_ids: Vec<ProjectId>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RequestedCapabilityGrant {
    pub capability: CapabilityKind,
    pub scope: String,
    pub reason: String,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

fn valid_package_version(value: &str) -> bool {
    let (core, suffix) = value
        .split_once('-')
        .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
    let mut parts = core.split('.');
    let valid_core = matches!((parts.next(), parts.next(), parts.next(), parts.next()), (Some(a), Some(b), Some(c), None) if [a,b,c].iter().all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())));
    valid_core
        && suffix.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        })
}

validated_contract!(
    CapabilityBundle, RawCapabilityBundle {
        schema_version: u16,
        contract_type: P1ContractType,
        capability_bundle_id: CapabilityBundleId,
        name: String,
        package_version: String,
        publisher: BundlePublisher,
        scope: BundleScope,
        capabilities: Vec<CapabilityKind>,
        requested_grants: Vec<RequestedCapabilityGrant>,
        payload_sha256: String,
        evidence: Vec<P1EvidenceReference>,
        signature: SignedEnvelope,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::CapabilityBundle)?;
        validate_string(&value.name, 1, 512, "capability bundle name")?;
        if !valid_package_version(&value.package_version) { return Err("invalid capability bundle package version".to_string()); }
        validate_string(&value.publisher.id, 1, 512, "publisher identifier")?;
        validate_string(&value.publisher.name, 1, 512, "publisher name")?;
        if value.scope.project_ids.len() > 1_024
            || (value.scope.kind == BundleScopeKind::Project && value.scope.project_ids.is_empty())
            || value.capabilities.len() > 256
            || value.requested_grants.len() > 256
            || value.evidence.len() > 4_096 {
            return Err("invalid capability bundle scope or collection size".to_string());
        }
        let allowed = value.scope.project_ids.iter().collect::<HashSet<_>>();
        if value.scope.kind == BundleScopeKind::Project && value.evidence.iter().any(|item| !allowed.contains(&item.project_id)) {
            return Err("cross-project capability bundle evidence".to_string());
        }
        for item in &value.evidence { item.validate()?; }
        let declared = value.capabilities.iter().copied().collect::<HashSet<_>>();
        for grant in &value.requested_grants {
            if !declared.contains(&grant.capability) { return Err("requested grant is not a declared capability".to_string()); }
            validate_string(&grant.scope, 1, 512, "capability grant scope")?;
            validate_string(&grant.reason, 1, 2_000, "capability grant reason")?;
        }
        validate_lower_hex(&value.payload_sha256, 32, "capability bundle payload digest")?;
        value.signature.validate()?;
        if value.payload_sha256 != value.signature.payload_sha256 { return Err("bundle signature digest mismatch".to_string()); }
        Ok(())
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningCandidateKind {
    Procedure,
    Preference,
    Correction,
    VerificationRule,
    FailureAvoidance,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningProposedScope {
    Project,
    GlobalWithConfirmation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningCandidateStatus {
    Proposed,
    Accepted,
    Rejected,
    Postponed,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningSource {
    pub project_id: ProjectId,
    pub task_id: TaskId,
    pub task_run_id: TaskRunId,
    pub evidence: Vec<P1EvidenceReference>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningProposedDiff {
    pub base: String,
    pub proposed: String,
    pub changed_fields: Vec<String>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

validated_contract!(
    LearningCandidate, RawLearningCandidate {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        learning_candidate_id: LearningCandidateId,
        candidate_version: u64,
        candidate_kind: LearningCandidateKind,
        proposed_scope: LearningProposedScope,
        summary: String,
        proposed_diff: LearningProposedDiff,
        status: LearningCandidateStatus,
        source_tasks: Vec<LearningSource>,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::LearningCandidate)?;
        if value.candidate_version == 0 || value.source_tasks.is_empty() || value.source_tasks.len() > 256 || value.evidence.is_empty() {
            return Err("invalid learning candidate version or evidence basis".to_string());
        }
        validate_string(&value.summary, 1, 4_000, "learning candidate summary")?;
        if value.proposed_diff.base.len() > 20_000 { return Err("learning diff base is too large".to_string()); }
        validate_string(&value.proposed_diff.proposed, 1, 20_000, "learning diff proposal")?;
        if value.proposed_diff.changed_fields.is_empty() || value.proposed_diff.changed_fields.len() > 256 { return Err("learning diff requires changed fields".to_string()); }
        for field in &value.proposed_diff.changed_fields { validate_string(field, 1, 512, "learning diff field")?; }
        for source in &value.source_tasks {
            if source.project_id != value.project_id || source.evidence.is_empty() { return Err("cross-project or unsupported learning source".to_string()); }
            validate_evidence_project(&value.project_id, &source.evidence)?;
        }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkGraphNodeKind {
    Parent,
    Specialist,
    Join,
    Checkpoint,
    Retry,
    Synthesis,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkGraphNode {
    pub node_id: String,
    pub kind: WorkGraphNodeKind,
    pub project_id: ProjectId,
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_run_id: Option<ChildRunId>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

fn graph_has_cycle(nodes: &[WorkGraphNode]) -> bool {
    fn visit(
        index: usize,
        nodes: &[WorkGraphNode],
        ids: &BTreeMap<&str, usize>,
        states: &mut [u8],
    ) -> bool {
        if states[index] == 1 {
            return true;
        }
        if states[index] == 2 {
            return false;
        }
        states[index] = 1;
        for dependency in &nodes[index].depends_on {
            if let Some(dependency_index) = ids.get(dependency.as_str()) {
                if visit(*dependency_index, nodes, ids, states) {
                    return true;
                }
            }
        }
        states[index] = 2;
        false
    }
    let ids = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.node_id.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut states = vec![0; nodes.len()];
    (0..nodes.len()).any(|index| visit(index, nodes, &ids, &mut states))
}

validated_contract!(
    WorkGraph, RawWorkGraph {
        schema_version: u16,
        contract_type: P1ContractType,
        project_id: ProjectId,
        task_id: TaskId,
        task_run_id: TaskRunId,
        work_graph_id: WorkGraphId,
        revision: u64,
        nodes: Vec<WorkGraphNode>,
        max_concurrent_children: u8,
        parent_owns_mutations: bool,
        resource_budget: ResourceBudgetTelemetry,
        evidence: Vec<P1EvidenceReference>,
        #[serde(flatten)] extensions: BTreeMap<String, Value>
    }
    validate |value| {
        validate_header(value.schema_version, value.contract_type, P1ContractType::WorkGraph)?;
        if value.revision == 0 || value.nodes.is_empty() || value.nodes.len() > 256
            || !(1..=8).contains(&value.max_concurrent_children) || !value.parent_owns_mutations {
            return Err("invalid work graph bounds or mutation ownership".to_string());
        }
        value.resource_budget.validate()?;
        if value.max_concurrent_children != value.resource_budget.limits.concurrent_children {
            return Err("work graph concurrency does not match its resource budget".to_string());
        }
        let mut ids = HashSet::new();
        for node in &value.nodes {
            if node.project_id != value.project_id || !ids.insert(node.node_id.as_str()) || node.depends_on.len() > 256 {
                return Err("invalid, duplicate, or cross-project work graph node".to_string());
            }
            validate_string(&node.node_id, 1, 512, "work graph node identifier")?;
        }
        for node in &value.nodes {
            if node.depends_on.iter().any(|dependency| dependency == &node.node_id || !ids.contains(dependency.as_str())) {
                return Err("work graph dependency is missing or self-referential".to_string());
            }
        }
        if graph_has_cycle(&value.nodes) { return Err("work graph dependency cycle".to_string()); }
        validate_evidence_project(&value.project_id, &value.evidence)
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!("../../schemas/p1-contract-v1-vectors.json"))
            .expect("shared P1 contract vectors must be valid JSON")
    }

    fn contract(name: &str) -> Value {
        vectors()["contracts"][name].clone()
    }

    fn assert_round_trip<T: DeserializeOwned + Serialize>(name: &str) {
        let fixture = contract(name);
        let parsed: T = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), fixture);
    }

    #[test]
    fn shared_vectors_round_trip_every_p1_contract() {
        assert_round_trip::<ArtifactWorkbook>("artifactWorkbook");
        assert_round_trip::<ArtifactPresentation>("artifactPresentation");
        assert_round_trip::<DesktopObservation>("desktopObservation");
        assert_round_trip::<DesktopAction>("desktopAction");
        assert_round_trip::<MediaAsset>("mediaAsset");
        assert_round_trip::<RemoteDevice>("remoteDevice");
        assert_round_trip::<CapabilityBundle>("capabilityBundle");
        assert_round_trip::<LearningCandidate>("learningCandidate");
        assert_round_trip::<WorkGraph>("workGraph");
    }

    #[test]
    fn zero_and_unknown_versions_fail_closed() {
        macro_rules! reject_versions {
            ($name:literal, $contract:ty) => {
                for version in [0, 2] {
                    let mut value = contract($name);
                    value["schemaVersion"] = json!(version);
                    assert!(serde_json::from_value::<$contract>(value).is_err());
                }
            };
        }

        reject_versions!("artifactWorkbook", ArtifactWorkbook);
        reject_versions!("artifactPresentation", ArtifactPresentation);
        reject_versions!("desktopObservation", DesktopObservation);
        reject_versions!("desktopAction", DesktopAction);
        reject_versions!("mediaAsset", MediaAsset);
        reject_versions!("remoteDevice", RemoteDevice);
        reject_versions!("capabilityBundle", CapabilityBundle);
        reject_versions!("learningCandidate", LearningCandidate);
        reject_versions!("workGraph", WorkGraph);
    }

    #[test]
    fn every_project_bound_contract_rejects_cross_project_references() {
        let cross = vectors()["idVectors"]["crossProject"].clone();

        let mut value = contract("artifactWorkbook");
        value["worksheets"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<ArtifactWorkbook>(value).is_err());

        let mut value = contract("artifactPresentation");
        value["slides"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<ArtifactPresentation>(value).is_err());

        let mut value = contract("desktopObservation");
        value["elements"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<DesktopObservation>(value).is_err());

        let mut value = contract("desktopAction");
        value["target"]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<DesktopAction>(value).is_err());

        let mut value = contract("mediaAsset");
        value["source"]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<MediaAsset>(value).is_err());

        let mut value = contract("remoteDevice");
        value["evidence"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<RemoteDevice>(value).is_err());

        let mut value = contract("capabilityBundle");
        value["evidence"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<CapabilityBundle>(value).is_err());

        let mut value = contract("learningCandidate");
        value["sourceTasks"][0]["projectId"] = cross.clone();
        assert!(serde_json::from_value::<LearningCandidate>(value).is_err());

        let mut value = contract("workGraph");
        value["nodes"][0]["projectId"] = cross;
        assert!(serde_json::from_value::<WorkGraph>(value).is_err());
    }

    #[test]
    fn unknown_evidence_vocabulary_fails_closed() {
        let mut value = contract("artifactWorkbook");
        value["evidence"][0]["evidenceClass"] = json!("claimed_success");
        assert!(serde_json::from_value::<ArtifactWorkbook>(value).is_err());
    }

    #[test]
    fn unsigned_remote_and_package_envelopes_fail_closed() {
        let mut device = contract("remoteDevice");
        device.as_object_mut().unwrap().remove("signature");
        assert!(serde_json::from_value::<RemoteDevice>(device).is_err());

        let mut bundle = contract("capabilityBundle");
        bundle.as_object_mut().unwrap().remove("signature");
        assert!(serde_json::from_value::<CapabilityBundle>(bundle).is_err());

        let mut bundle = contract("capabilityBundle");
        bundle["signature"]["payloadSha256"] = json!("0".repeat(64));
        assert!(serde_json::from_value::<CapabilityBundle>(bundle).is_err());
    }

    #[test]
    fn forward_optional_fields_are_preserved() {
        let mut value = contract("artifactPresentation");
        value["futureOptional"] = json!({"reviewHint": true});
        value["slides"][0]["futureSlideHint"] = json!("safe");
        let parsed: ArtifactPresentation = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }

    #[test]
    fn work_graph_cycles_and_budget_mismatches_fail_closed() {
        let mut graph = contract("workGraph");
        graph["nodes"][0]["dependsOn"] = json!(["synthesis"]);
        assert!(serde_json::from_value::<WorkGraph>(graph).is_err());

        let mut graph = contract("workGraph");
        graph["resourceBudget"]["usage"]["peakConcurrentChildren"] = json!(9);
        assert!(serde_json::from_value::<WorkGraph>(graph).is_err());

        let mut graph = contract("workGraph");
        graph["resourceBudget"]["usage"]["mutationsCommitted"] = json!(2);
        graph["resourceBudget"]["usage"]["mutationAttempts"] = json!(1);
        assert!(serde_json::from_value::<WorkGraph>(graph).is_err());
    }
}
