use crate::{
    db::ChatTurnPersistenceContext,
    macos_permission_broker::{status_for_operation, MacosPermissionState, MacosPermissionStatus},
    mcp_result::McpToolCallResult,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

const RECEIPT_SCHEMA_VERSION: u8 = 1;
const MAX_EMITTED_BINDINGS: usize = 512;
const MAX_CONTINUATION_RECEIPTS: usize = 512;
mod capability_contract;
mod read_evidence;
pub(crate) use capability_contract::{
    contract_is_complete, ActionApprovalClass, AppleCapability, NativeActionClass,
    NativeOperationOutcome,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePostconditionEvidence {
    pub evidence_kind: &'static str,
    pub operation_succeeded: bool,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounded_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_result_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durable_operation_binding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_proof: Option<NativeScreenCaptureProof>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeScreenCaptureProof {
    pub method: &'static str,
    pub requesting_process_id: u32,
    pub captured_window_count: usize,
    pub width: u32,
    pub height: u32,
    pub png_byte_count: usize,
    pub pixel_digest_sha256: String,
    pub non_uniform_pixels: bool,
    pub retained_byte_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatTurnOrigin {
    session_id: String,
    turn_id: String,
    root_turn_id: String,
    generation_token_sha256: String,
    agent_id_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkflowExecutionOrigin {
    execution_id: String,
    workflow_id: String,
    workflow_version: u32,
    node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_binding_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NativeOperationOrigin {
    ChatTurn(ChatTurnOrigin),
    WorkflowExecution(WorkflowExecutionOrigin),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionEvidence {
    state: MacosPermissionState,
    can_request: bool,
    authority_owner: String,
    framework: String,
}

impl From<MacosPermissionStatus> for PermissionEvidence {
    fn from(value: MacosPermissionStatus) -> Self {
        Self {
            state: value.state,
            can_request: value.can_request,
            authority_owner: value.authority_owner,
            framework: value.framework,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeAppleOperationReceipt {
    schema_version: u8,
    receipt_id: String,
    capability_id: &'static str,
    action_class: NativeActionClass,
    approval_class: ActionApprovalClass,
    action_approved: bool,
    action_binding: String,
    execution_binding_sha256: String,
    origin: NativeOperationOrigin,
    requesting_process: crate::macos_process_identity::MacosProcessIdentityEvidence,
    authority_owner: &'static str,
    framework: &'static str,
    permission_before: PermissionEvidence,
    permission_after: PermissionEvidence,
    outcome: NativeOperationOutcome,
    result: NativePostconditionEvidence,
    exactly_once: bool,
    marker_first_emission_this_process: bool,
    recorded_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConsumedNativeExecutionReceipt {
    pub receipt_id: String,
    pub execution_binding_sha256: String,
    pub capability_id: String,
    pub action_class: NativeActionClass,
    pub outcome: NativeOperationOutcome,
    pub verified_success: bool,
    pub native_result_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeReceiptConsumptionError {
    InvalidReceipt,
    ContextMismatch,
    WorkflowReceipt,
    Replayed,
}

impl NativeReceiptConsumptionError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::InvalidReceipt => "native_execution_receipt_invalid",
            Self::ContextMismatch => "native_execution_receipt_context_mismatch",
            Self::WorkflowReceipt => "native_execution_receipt_not_chat_bound",
            Self::Replayed => "native_execution_receipt_replayed",
        }
    }
}

impl fmt::Display for NativeReceiptConsumptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

#[derive(Clone, Debug)]
enum ContinuationReceiptOrigin {
    ChatTurn {
        session_id: String,
        turn_id: String,
        root_turn_id: String,
        generation_token_sha256: String,
        agent_id_sha256: String,
    },
    Workflow,
}

#[derive(Clone, Debug)]
struct ContinuationReceiptAuthority {
    receipt_id: String,
    execution_binding_sha256: String,
    capability_id: String,
    action_class: NativeActionClass,
    outcome: NativeOperationOutcome,
    verified_success: bool,
    native_result_code: Option<String>,
    origin: ContinuationReceiptOrigin,
    consumed: bool,
}

#[derive(Default)]
struct ContinuationReceiptLedger {
    entries: HashMap<String, ContinuationReceiptAuthority>,
    insertion_order: VecDeque<String>,
}

pub(crate) struct NativeOperationAttempt {
    capability: AppleCapability,
    action: NativeActionClass,
    approval_class: ActionApprovalClass,
    action_approved: bool,
    action_binding: String,
    origin: NativeOperationOrigin,
    permission_before: PermissionEvidence,
    persistence: Option<crate::db::PersistenceEngine>,
    turn_context: Option<ChatTurnPersistenceContext>,
}

impl NativeOperationAttempt {
    pub(crate) async fn begin(
        capability: AppleCapability,
        action: NativeActionClass,
        action_approved: bool,
        action_binding: String,
        turn: Option<&ChatTurnPersistenceContext>,
    ) -> Option<Self> {
        let origin = turn
            .map(ChatTurnOrigin::from_context)
            .map(NativeOperationOrigin::ChatTurn)?;
        Self::begin_with_origin(
            capability,
            action,
            action_approved,
            action_binding,
            origin,
            None,
            turn.cloned(),
        )
        .await
    }

    pub(crate) async fn begin_with_persistence(
        capability: AppleCapability,
        action: NativeActionClass,
        action_approved: bool,
        action_binding: String,
        turn: Option<&ChatTurnPersistenceContext>,
        persistence: &crate::db::PersistenceEngine,
    ) -> Option<Self> {
        let turn = turn?;
        let origin = NativeOperationOrigin::ChatTurn(ChatTurnOrigin::from_context(turn));
        let before = status_for_operation(capability.descriptor().id).await;
        if usable_permission(before.state) {
            persistence
                .prepare_permission_turn_retry(turn, capability.descriptor().id)
                .ok()?;
        }
        Self::begin_with_origin(
            capability,
            action,
            action_approved,
            action_binding,
            origin,
            Some(persistence.clone()),
            Some(turn.clone()),
        )
        .await
    }

    async fn begin_with_origin(
        capability: AppleCapability,
        action: NativeActionClass,
        action_approved: bool,
        action_binding: String,
        origin: NativeOperationOrigin,
        persistence: Option<crate::db::PersistenceEngine>,
        turn_context: Option<ChatTurnPersistenceContext>,
    ) -> Option<Self> {
        let approval_class = approval_class(capability, action);
        let before = status_for_operation(capability.descriptor().id).await;
        Some(Self {
            capability,
            action,
            approval_class,
            action_approved,
            action_binding,
            origin,
            permission_before: before.into(),
            persistence,
            turn_context,
        })
    }

    pub(crate) async fn begin_for_execution(
        capability: AppleCapability,
        action: NativeActionClass,
        action_approved: bool,
        action_binding: String,
        persistence: &crate::db::PersistenceEngine,
        execution_id: &str,
    ) -> Option<Self> {
        let turn = accepted_turn_for_execution(persistence, execution_id)?;
        persistence.ensure_chat_turn_for_native_action(&turn).ok()?;
        Self::begin_with_persistence(
            capability,
            action,
            action_approved,
            action_binding,
            Some(&turn),
            persistence,
        )
        .await
    }

    pub(crate) async fn begin_for_registered_task_execution(
        capability: AppleCapability,
        action: NativeActionClass,
        action_binding: String,
        persistence: &crate::db::PersistenceEngine,
        execution_id: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Option<Self> {
        if let Some(attempt) = Self::begin_for_execution(
            capability,
            action,
            true,
            action_binding.clone(),
            persistence,
            execution_id,
        )
        .await
        {
            return Some(attempt);
        }
        Self::begin_for_workflow_execution(
            capability,
            action,
            true,
            action_binding,
            persistence,
            execution_id,
            "oomu_task_tools",
            tool_name,
            arguments,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn begin_for_workflow_execution(
        capability: AppleCapability,
        action: NativeActionClass,
        action_approved: bool,
        action_binding: String,
        persistence: &crate::db::PersistenceEngine,
        execution_id: &str,
        server_name: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Option<Self> {
        let approval_required = matches!(
            approval_class(capability, action),
            ActionApprovalClass::ExplicitAction | ActionApprovalClass::ExplicitHighImpact
        );
        let origin = accepted_workflow_origin(
            persistence,
            execution_id,
            server_name,
            tool_name,
            arguments,
            approval_required,
            action_approved,
        )?;
        Self::begin_with_origin(
            capability,
            action,
            action_approved,
            action_binding,
            NativeOperationOrigin::WorkflowExecution(origin),
            None,
            None,
        )
        .await
    }

    pub(crate) async fn finish(
        self,
        result: NativePostconditionEvidence,
    ) -> NativeAppleOperationReceipt {
        let descriptor = self.capability.descriptor();
        let permission_after: PermissionEvidence = status_for_operation(descriptor.id).await.into();
        let approval_met = !matches!(
            self.approval_class,
            ActionApprovalClass::ExplicitAction | ActionApprovalClass::ExplicitHighImpact
        ) || self.action_approved;
        let permission_met = usable_permission(permission_after.state);
        let supported = self.capability.supports(self.action)
            && self.approval_class != ActionApprovalClass::Unsupported;
        let outcome = native_operation_outcome(supported, approval_met, permission_met, &result);
        let binding = emission_binding(
            &self.origin,
            descriptor.id,
            self.action,
            &self.action_binding,
        );
        let marker_first_emission_this_process = claim_emission(binding.clone());
        let exactly_once = result.durable_operation_binding.is_some();
        let receipt_binding = result
            .durable_operation_binding
            .as_deref()
            .unwrap_or(&binding);
        let receipt_suffix = receipt_id_suffix(receipt_binding);
        let continuation = self.persistence.clone().zip(self.turn_context.clone());
        let permission_after_state = permission_after.state;
        let receipt = NativeAppleOperationReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            receipt_id: next_receipt_id(&receipt_suffix),
            capability_id: descriptor.id,
            action_class: self.action,
            approval_class: self.approval_class,
            action_approved: self.action_approved,
            action_binding: self.action_binding,
            execution_binding_sha256: binding,
            origin: self.origin,
            requesting_process: crate::macos_process_identity::current(),
            authority_owner: descriptor.authority_owner,
            framework: descriptor.framework,
            permission_before: self.permission_before,
            permission_after,
            outcome,
            result,
            exactly_once,
            marker_first_emission_this_process,
            recorded_at_ms: crate::foundation::clock::unix_time_ms_i64(),
        };
        register_terminal_receipt(&receipt);
        if let Some((persistence, turn)) = continuation {
            match receipt.outcome {
                NativeOperationOutcome::Unmet if !permission_met => {
                    let _ = persistence.pause_permission_turn_for_native_receipt(
                        &turn,
                        descriptor.id,
                        &receipt.receipt_id,
                        permission_after_state,
                        receipt.result.native_result_code.as_deref(),
                    );
                }
                NativeOperationOutcome::Succeeded => {
                    if let Ok(Some(continued)) = persistence.complete_permission_turn_continuation(
                        &turn,
                        descriptor.id,
                        &receipt.receipt_id,
                        permission_after_state,
                    ) {
                        if crate::diagnostic_output::native_acceptance_enabled() {
                            if let Ok(encoded) = serde_json::to_string(&continued) {
                                eprintln!("OOMU_NATIVE_RECEIPT {encoded}");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if marker_first_emission_this_process
            && crate::diagnostic_output::native_acceptance_enabled()
        {
            if let Ok(encoded) = serde_json::to_string(&receipt) {
                eprintln!("OOMU_APPLE_OPERATION_RECEIPT {encoded}");
            }
        }
        receipt
    }

    pub(crate) fn action_was_approved(mut self) -> Self {
        self.action_approved = true;
        self
    }
}

impl NativeAppleOperationReceipt {
    pub(crate) fn verified_success(&self) -> bool {
        self.outcome == NativeOperationOutcome::Succeeded && self.result.verified
    }

    /// Attach only the privacy-bounded, native-authored portion of this receipt
    /// to an MCP result. The full receipt retains turn and permission details for
    /// native diagnostics; the renderer receives an opaque execution binding and
    /// the terminal facts needed to continue the model turn truthfully.
    pub(crate) fn attach_to_mcp_result(&self, result: &mut McpToolCallResult) {
        let projection = serde_json::json!({
            "schema": "oomu.native-mcp-execution.v1",
            "receiptId": self.receipt_id,
            "continuationAuthority": "process_owned_one_use",
            "executionBindingSha256": self.execution_binding_sha256,
            "capabilityId": self.capability_id,
            "actionClass": self.action_class,
            "outcome": self.outcome,
            "verified": self.verified_success(),
            "postcondition": {
                "evidenceKind": self.result.evidence_kind,
                "operationSucceeded": self.result.operation_succeeded,
                "verified": self.result.verified,
                "nativeResultCode": self.result.native_result_code,
            },
            "exactlyOnce": self.exactly_once,
            "recordedAtMs": self.recorded_at_ms,
        });
        let prior = result.meta.take();
        let mut metadata = match prior {
            Some(Value::Object(metadata)) => metadata,
            Some(upstream) => {
                let mut metadata = serde_json::Map::new();
                metadata.insert("upstream".to_string(), upstream);
                metadata
            }
            None => serde_json::Map::new(),
        };
        metadata.insert("oomuNativeExecutionReceipt".to_string(), projection);
        result.meta = Some(Value::Object(metadata));
    }
}

impl ContinuationReceiptLedger {
    fn register(&mut self, receipt: &NativeAppleOperationReceipt) {
        if self.entries.contains_key(&receipt.receipt_id) {
            return;
        }
        let origin = match &receipt.origin {
            NativeOperationOrigin::ChatTurn(turn) => ContinuationReceiptOrigin::ChatTurn {
                session_id: turn.session_id.clone(),
                turn_id: turn.turn_id.clone(),
                root_turn_id: turn.root_turn_id.clone(),
                generation_token_sha256: turn.generation_token_sha256.clone(),
                agent_id_sha256: turn.agent_id_sha256.clone(),
            },
            NativeOperationOrigin::WorkflowExecution(_) => ContinuationReceiptOrigin::Workflow,
        };
        let authority = ContinuationReceiptAuthority {
            receipt_id: receipt.receipt_id.clone(),
            execution_binding_sha256: receipt.execution_binding_sha256.clone(),
            capability_id: receipt.capability_id.to_string(),
            action_class: receipt.action_class,
            outcome: receipt.outcome,
            verified_success: receipt.outcome == NativeOperationOutcome::Succeeded
                && receipt.result.verified,
            native_result_code: receipt.result.native_result_code.clone(),
            origin,
            consumed: false,
        };
        self.insertion_order.push_back(receipt.receipt_id.clone());
        self.entries.insert(receipt.receipt_id.clone(), authority);
        while self.insertion_order.len() > MAX_CONTINUATION_RECEIPTS {
            if let Some(expired) = self.insertion_order.pop_front() {
                self.entries.remove(&expired);
            }
        }
    }

    fn validate(
        &self,
        receipt_id: &str,
        parent_context: &ChatTurnPersistenceContext,
    ) -> Result<(), NativeReceiptConsumptionError> {
        let normalized_receipt_id = receipt_id.trim();
        if normalized_receipt_id != receipt_id
            || normalized_receipt_id.is_empty()
            || normalized_receipt_id.len() > 240
            || !normalized_receipt_id.starts_with("apple-operation-")
            || !normalized_receipt_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(NativeReceiptConsumptionError::InvalidReceipt);
        }
        let authority = self
            .entries
            .get(normalized_receipt_id)
            .ok_or(NativeReceiptConsumptionError::InvalidReceipt)?;
        let ContinuationReceiptOrigin::ChatTurn {
            session_id,
            turn_id,
            root_turn_id,
            generation_token_sha256,
            agent_id_sha256,
        } = &authority.origin
        else {
            return Err(NativeReceiptConsumptionError::WorkflowReceipt);
        };
        let context_matches = session_id == &parent_context.session_id
            && turn_id == &parent_context.turn_id
            && root_turn_id == &parent_context.root_turn_id
            && generation_token_sha256
                == &crate::foundation::digest::sha256_hex(
                    parent_context.generation_token.as_bytes(),
                )
            && agent_id_sha256
                == &crate::foundation::digest::sha256_hex(parent_context.agent_id.as_bytes());
        if !context_matches {
            return Err(NativeReceiptConsumptionError::ContextMismatch);
        }
        if authority.consumed {
            return Err(NativeReceiptConsumptionError::Replayed);
        }
        Ok(())
    }

    fn consume(
        &mut self,
        receipt_id: &str,
        parent_context: &ChatTurnPersistenceContext,
    ) -> Result<ConsumedNativeExecutionReceipt, NativeReceiptConsumptionError> {
        self.validate(receipt_id, parent_context)?;
        let authority = self
            .entries
            .get_mut(receipt_id)
            .ok_or(NativeReceiptConsumptionError::InvalidReceipt)?;
        authority.consumed = true;
        Ok(ConsumedNativeExecutionReceipt {
            receipt_id: authority.receipt_id.clone(),
            execution_binding_sha256: authority.execution_binding_sha256.clone(),
            capability_id: authority.capability_id.clone(),
            action_class: authority.action_class,
            outcome: authority.outcome,
            verified_success: authority.verified_success,
            native_result_code: authority.native_result_code.clone(),
        })
    }
}

pub(crate) fn validate_chat_turn_receipt(
    receipt_id: &str,
    parent_context: &ChatTurnPersistenceContext,
) -> Result<(), NativeReceiptConsumptionError> {
    let ledger = continuation_receipt_ledger();
    let ledger = ledger
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ledger.validate(receipt_id, parent_context)
}

pub(crate) fn consume_chat_turn_receipt(
    receipt_id: &str,
    parent_context: &ChatTurnPersistenceContext,
) -> Result<ConsumedNativeExecutionReceipt, NativeReceiptConsumptionError> {
    let ledger = continuation_receipt_ledger();
    let mut ledger = ledger
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ledger.consume(receipt_id, parent_context)
}

fn continuation_receipt_ledger() -> &'static Mutex<ContinuationReceiptLedger> {
    static LEDGER: OnceLock<Mutex<ContinuationReceiptLedger>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(ContinuationReceiptLedger::default()))
}

fn register_terminal_receipt(receipt: &NativeAppleOperationReceipt) {
    let ledger = continuation_receipt_ledger();
    let mut ledger = ledger
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    ledger.register(receipt);
}

fn next_receipt_id(receipt_suffix: &str) -> String {
    static RECEIPT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let sequence = RECEIPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!(
        "apple-operation-{}-{}-{sequence:x}",
        crate::foundation::clock::unix_time_ms_i64().max(0),
        receipt_suffix
    )
}

fn native_operation_outcome(
    supported: bool,
    approval_met: bool,
    permission_met: bool,
    result: &NativePostconditionEvidence,
) -> NativeOperationOutcome {
    if !supported {
        NativeOperationOutcome::Unsupported
    } else if !approval_met || !permission_met {
        NativeOperationOutcome::Unmet
    } else if !result.operation_succeeded {
        NativeOperationOutcome::Failed
    } else if !result.verified {
        NativeOperationOutcome::Unmet
    } else {
        NativeOperationOutcome::Succeeded
    }
}

fn receipt_id_suffix(binding: &str) -> String {
    crate::foundation::digest::sha256_hex(binding.as_bytes())[..16].to_string()
}

fn accepted_turn_for_execution(
    persistence: &crate::db::PersistenceEngine,
    execution_id: &str,
) -> Option<ChatTurnPersistenceContext> {
    persistence
        .open_connection()
        .ok()?
        .query_row(
            "SELECT turn_id,generation_token,session_id,agent_id,provider_id,model_id,parent_turn_id,root_turn_id,turn_kind FROM agent_executions WHERE execution_id=?1",
            rusqlite::params![execution_id],
            |row| {
                Ok(ChatTurnPersistenceContext {
                    turn_id: row.get(0)?,
                    generation_token: row.get(1)?,
                    session_id: row.get(2)?,
                    agent_id: row.get(3)?,
                    provider_id: row.get(4)?,
                    model_id: row.get(5)?,
                    parent_turn_id: row.get(6)?,
                    root_turn_id: row.get(7)?,
                    turn_kind: row.get(8)?,
                })
            },
        )
        .ok()
}

impl ChatTurnOrigin {
    fn from_context(context: &ChatTurnPersistenceContext) -> Self {
        Self {
            session_id: context.session_id.clone(),
            turn_id: context.turn_id.clone(),
            root_turn_id: context.root_turn_id.clone(),
            generation_token_sha256: crate::foundation::digest::sha256_hex(
                context.generation_token.as_bytes(),
            ),
            agent_id_sha256: crate::foundation::digest::sha256_hex(context.agent_id.as_bytes()),
        }
    }
}

fn accepted_workflow_origin(
    persistence: &crate::db::PersistenceEngine,
    execution_id: &str,
    server_name: &str,
    tool_name: &str,
    arguments: &Value,
    approval_required: bool,
    action_approved: bool,
) -> Option<WorkflowExecutionOrigin> {
    if approval_required && !action_approved {
        return None;
    }
    let execution_id = execution_id.trim();
    let server_name = server_name.trim();
    let tool_name = tool_name.trim();
    if execution_id.is_empty() || server_name.is_empty() || tool_name.is_empty() {
        return None;
    }
    let connection = persistence.open_connection().ok()?;
    let (workflow_id, workflow_version, node_id, status) = connection
        .query_row(
            "SELECT workflow_id,workflow_version,active_node_id,status FROM execution_instances WHERE id=?1",
            rusqlite::params![execution_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .ok()?;
    let node_id = node_id?.trim().to_string();
    if node_id.is_empty() || !matches!(status.as_str(), "Running" | "AwaitingApproval") {
        return None;
    }
    if !approval_required {
        return Some(WorkflowExecutionOrigin {
            execution_id: execution_id.to_string(),
            workflow_id,
            workflow_version,
            node_id,
            approval_node_id: None,
            approval_binding_sha256: None,
        });
    }
    let exact_material = serde_json::json!({
        "arguments": arguments,
        "approvalBinding": Value::Null,
    });
    let version_material = serde_json::json!({
        "schema": "workflow_version_mcp_review_v1",
        "serverName": server_name,
        "toolName": tool_name,
        "exactCall": exact_material,
    });
    let exact_hash = crate::db::hash_arguments(
        version_material
            .get("exactCall")
            .expect("exact Workflow approval material exists"),
    );
    let version_hash = crate::db::hash_arguments(&version_material);
    let version_subject = format!("saved-workflow:{workflow_id}:version:{workflow_version}");
    let version_target = serde_json::to_string(&[server_name, tool_name]).ok()?;
    let now_seconds = crate::foundation::clock::unix_time_ms_i64().max(0) / 1_000;
    let approval = connection
        .query_row(
            "SELECT approval_token,node_id,workflow_instance_id,target_tool_name,arguments_hash FROM workflow_approvals WHERE decision='approve' AND expires_at>?1 AND ((workflow_instance_id=?2 AND target_tool_name=?3 AND arguments_hash=?4) OR (workflow_instance_id=?5 AND node_id=?6 AND target_tool_name=?7 AND arguments_hash=?8)) ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![
                now_seconds,
                execution_id,
                tool_name,
                exact_hash,
                version_subject,
                node_id,
                version_target,
                version_hash,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .ok()??;
    let approval_binding_sha256 = crate::foundation::digest::sha256_hex(
        format!(
            "workflow-native-operation-v1\0{}\0{}\0{}\0{}\0{}",
            approval.0, approval.1, approval.2, approval.3, approval.4
        )
        .as_bytes(),
    );
    Some(WorkflowExecutionOrigin {
        execution_id: execution_id.to_string(),
        workflow_id,
        workflow_version,
        node_id,
        approval_node_id: Some(approval.1),
        approval_binding_sha256: Some(approval_binding_sha256),
    })
}

pub(crate) fn evidence_from_mcp_result(
    action: NativeActionClass,
    result: &McpToolCallResult,
) -> NativePostconditionEvidence {
    let structured = result.structured_content.as_ref();
    let result_code = structured
        .and_then(|value| value.get("code").or_else(|| value.get("status")))
        .and_then(Value::as_str)
        .map(|value| value.chars().take(80).collect());
    let (kind, count) = read_evidence::from_structured(structured);
    let explicit_verified = structured
        .and_then(|value| value.get("verified"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let native_object_returned = structured
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    let durable_operation_binding = durable_native_operation_binding(action, structured);
    let verified = !result.is_error
        && match action {
            NativeActionClass::Read => count.is_some(),
            NativeActionClass::Write | NativeActionClass::Draft => {
                explicit_verified || native_object_returned
            }
            NativeActionClass::Delete => structured
                .and_then(|value| value.get("deleted"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            _ => explicit_verified,
        };
    NativePostconditionEvidence {
        evidence_kind: kind,
        operation_succeeded: !result.is_error,
        verified,
        bounded_count: count,
        truncated: structured
            .and_then(|value| value.get("truncated"))
            .and_then(Value::as_bool),
        native_result_code: result_code,
        durable_operation_binding,
        capture_proof: None,
    }
}

fn durable_native_operation_binding(
    action: NativeActionClass,
    structured: Option<&Value>,
) -> Option<String> {
    if matches!(
        action,
        NativeActionClass::Read
            | NativeActionClass::Observe
            | NativeActionClass::Probe
            | NativeActionClass::Capture
            | NativeActionClass::Notify
    ) {
        return None;
    }
    let value = structured?;
    for key in ["draftId", "messageId", "noteId", "reminderId", "id"] {
        if let Some(identifier) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|identifier| !identifier.is_empty())
        {
            return Some(crate::foundation::digest::sha256_hex(
                format!("native-apple-effect\0{key}\0{identifier}").as_bytes(),
            ));
        }
    }
    None
}

fn approval_class(capability: AppleCapability, action: NativeActionClass) -> ActionApprovalClass {
    if !capability.supports(action) {
        return ActionApprovalClass::Unsupported;
    }
    match action {
        NativeActionClass::Read | NativeActionClass::Observe | NativeActionClass::Probe => {
            if matches!(
                capability,
                AppleCapability::Finder
                    | AppleCapability::FilesAndFolders
                    | AppleCapability::FullDiskAccess
            ) {
                ActionApprovalClass::ResourceScope
            } else {
                ActionApprovalClass::NativePermissionOnly
            }
        }
        NativeActionClass::Send | NativeActionClass::Delete => {
            ActionApprovalClass::ExplicitHighImpact
        }
        _ => ActionApprovalClass::ExplicitAction,
    }
}

fn usable_permission(state: MacosPermissionState) -> bool {
    matches!(
        state,
        MacosPermissionState::Allowed
            | MacosPermissionState::Limited
            | MacosPermissionState::WhenUsed
    )
}

fn emission_binding(
    origin: &NativeOperationOrigin,
    capability_id: &str,
    action: NativeActionClass,
    action_binding: &str,
) -> String {
    let origin_binding = match origin {
        NativeOperationOrigin::ChatTurn(turn) => format!(
            "chat\0{}\0{}\0{}\0{}",
            turn.session_id, turn.turn_id, turn.generation_token_sha256, turn.agent_id_sha256
        ),
        NativeOperationOrigin::WorkflowExecution(workflow) => format!(
            "workflow\0{}\0{}\0{}",
            workflow.execution_id, workflow.node_id, workflow.workflow_version
        ),
    };
    crate::foundation::digest::sha256_hex(
        format!(
            "{}\0{}\0{}\0{}",
            origin_binding, capability_id, action as u8, action_binding
        )
        .as_bytes(),
    )
}

fn claim_emission(binding: String) -> bool {
    static EMITTED: OnceLock<Mutex<(HashSet<String>, VecDeque<String>)>> = OnceLock::new();
    let Ok(mut state) = EMITTED
        .get_or_init(|| Mutex::new((HashSet::new(), VecDeque::new())))
        .lock()
    else {
        return false;
    };
    if !state.0.insert(binding.clone()) {
        return false;
    }
    state.1.push_back(binding);
    while state.1.len() > MAX_EMITTED_BINDINGS {
        if let Some(expired) = state.1.pop_front() {
            state.0.remove(&expired);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt_fixture(
        outcome: NativeOperationOutcome,
        result: NativePostconditionEvidence,
    ) -> NativeAppleOperationReceipt {
        NativeAppleOperationReceipt {
            schema_version: RECEIPT_SCHEMA_VERSION,
            receipt_id: "apple-operation-test".to_string(),
            capability_id: "calendar",
            action_class: NativeActionClass::Read,
            approval_class: ActionApprovalClass::NativePermissionOnly,
            action_approved: true,
            action_binding: "private-title".to_string(),
            execution_binding_sha256: crate::foundation::digest::sha256_hex(
                b"session-secret\0turn-secret\0generation-secret\0private-title",
            ),
            origin: NativeOperationOrigin::ChatTurn(ChatTurnOrigin {
                session_id: "session-secret".to_string(),
                turn_id: "turn-secret".to_string(),
                root_turn_id: "root-secret".to_string(),
                generation_token_sha256: crate::foundation::digest::sha256_hex(
                    b"generation-secret",
                ),
                agent_id_sha256: crate::foundation::digest::sha256_hex(b"agent-secret"),
            }),
            requesting_process: crate::macos_process_identity::current(),
            authority_owner: "main_app",
            framework: "EventKit",
            permission_before: PermissionEvidence {
                state: MacosPermissionState::Allowed,
                can_request: false,
                authority_owner: "main_app".to_string(),
                framework: "EventKit".to_string(),
            },
            permission_after: PermissionEvidence {
                state: MacosPermissionState::Allowed,
                can_request: false,
                authority_owner: "main_app".to_string(),
                framework: "EventKit".to_string(),
            },
            outcome,
            result,
            exactly_once: false,
            marker_first_emission_this_process: true,
            recorded_at_ms: 1_234,
        }
    }

    fn receipt_parent_context() -> ChatTurnPersistenceContext {
        ChatTurnPersistenceContext {
            turn_id: "turn-secret".to_string(),
            generation_token: "generation-secret".to_string(),
            session_id: "session-secret".to_string(),
            agent_id: "agent-secret".to_string(),
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
            parent_turn_id: None,
            root_turn_id: "root-secret".to_string(),
            turn_kind: "user".to_string(),
        }
    }

    #[test]
    fn capability_contract_covers_every_required_row_once() {
        let ids = AppleCapability::ALL
            .into_iter()
            .map(|capability| capability.descriptor().id)
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), 20);
        assert!(ids.contains("calendar"));
        assert!(ids.contains("full_disk_access"));
    }

    #[test]
    fn read_grants_do_not_authorize_mutations() {
        assert_eq!(
            approval_class(AppleCapability::Notes, NativeActionClass::Read),
            ActionApprovalClass::NativePermissionOnly
        );
        assert_eq!(
            approval_class(AppleCapability::Notes, NativeActionClass::Write),
            ActionApprovalClass::ExplicitAction
        );
        assert_eq!(
            approval_class(AppleCapability::Mail, NativeActionClass::Send),
            ActionApprovalClass::ExplicitHighImpact
        );
    }

    #[test]
    fn revoked_native_permission_is_unmet_not_a_false_execution_failure() {
        let evidence = NativePostconditionEvidence {
            evidence_kind: "native_call_error",
            operation_succeeded: false,
            verified: false,
            bounded_count: None,
            truncated: None,
            native_result_code: Some("permission_required".to_string()),
            durable_operation_binding: None,
            capture_proof: None,
        };
        assert_eq!(
            native_operation_outcome(true, true, false, &evidence),
            NativeOperationOutcome::Unmet
        );
        assert_eq!(
            native_operation_outcome(true, true, true, &evidence),
            NativeOperationOutcome::Failed
        );
    }

    #[test]
    fn receipt_shape_cannot_store_native_private_content() {
        let serialized = serde_json::to_value(NativePostconditionEvidence {
            evidence_kind: "bounded_native_collection",
            operation_succeeded: true,
            verified: true,
            bounded_count: Some(1),
            truncated: Some(false),
            native_result_code: Some("ok".to_string()),
            durable_operation_binding: None,
            capture_proof: None,
        })
        .unwrap();
        let encoded = serialized.to_string();
        for forbidden in ["title", "body", "path", "recipient", "contact", "message"] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn mcp_projection_is_bounded_and_preserves_upstream_metadata() {
        let receipt = receipt_fixture(
            NativeOperationOutcome::Succeeded,
            NativePostconditionEvidence {
                evidence_kind: "bounded_native_collection",
                operation_succeeded: true,
                verified: true,
                bounded_count: Some(1),
                truncated: Some(false),
                native_result_code: Some("calendar_read_ok".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut result = McpToolCallResult {
            content: Vec::new(),
            structured_content: Some(serde_json::json!({"events": []})),
            is_error: false,
            meta: Some(serde_json::json!({"upstreamKey": "retained"})),
            raw: None,
        };

        receipt.attach_to_mcp_result(&mut result);

        let meta = result.meta.unwrap();
        assert_eq!(meta["upstreamKey"], "retained");
        let native = &meta["oomuNativeExecutionReceipt"];
        assert_eq!(native["schema"], "oomu.native-mcp-execution.v1");
        assert_eq!(native["receiptId"], "apple-operation-test");
        assert_eq!(native["outcome"], "succeeded");
        assert_eq!(native["verified"], true);
        assert_eq!(native["postcondition"]["verified"], true);
        assert_eq!(
            native["executionBindingSha256"].as_str().map(str::len),
            Some(64)
        );
        let encoded = native.to_string();
        for forbidden in [
            "session-secret",
            "turn-secret",
            "generation-secret",
            "private-title",
        ] {
            assert!(!encoded.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn mcp_projection_reports_typed_denial_without_claiming_verification() {
        let receipt = receipt_fixture(
            NativeOperationOutcome::Unmet,
            NativePostconditionEvidence {
                evidence_kind: "native_result",
                operation_succeeded: false,
                verified: false,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("calendar_permission_denied".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut result = McpToolCallResult {
            content: Vec::new(),
            structured_content: Some(serde_json::json!({
                "code": "calendar_permission_denied"
            })),
            is_error: true,
            meta: None,
            raw: None,
        };

        receipt.attach_to_mcp_result(&mut result);

        let native = &result.meta.unwrap()["oomuNativeExecutionReceipt"];
        assert_eq!(native["outcome"], "unmet");
        assert_eq!(native["verified"], false);
        assert_eq!(
            native["postcondition"]["nativeResultCode"],
            "calendar_permission_denied"
        );
    }

    #[test]
    fn continuation_authority_is_context_bound_and_one_use() {
        let receipt = receipt_fixture(
            NativeOperationOutcome::Succeeded,
            NativePostconditionEvidence {
                evidence_kind: "bounded_native_collection",
                operation_succeeded: true,
                verified: true,
                bounded_count: Some(0),
                truncated: Some(false),
                native_result_code: Some("calendar_read_ok".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut ledger = ContinuationReceiptLedger::default();
        ledger.register(&receipt);

        let consumed = ledger
            .consume(&receipt.receipt_id, &receipt_parent_context())
            .unwrap();
        assert_eq!(consumed.receipt_id, receipt.receipt_id);
        assert_eq!(consumed.capability_id, "calendar");
        assert_eq!(consumed.action_class, NativeActionClass::Read);
        assert_eq!(consumed.outcome, NativeOperationOutcome::Succeeded);
        assert!(consumed.verified_success);
        assert_eq!(
            consumed.native_result_code.as_deref(),
            Some("calendar_read_ok")
        );
        assert_eq!(consumed.execution_binding_sha256.len(), 64);
        assert_eq!(
            ledger.consume(&receipt.receipt_id, &receipt_parent_context()),
            Err(NativeReceiptConsumptionError::Replayed)
        );
    }

    #[test]
    fn continuation_authority_validation_does_not_consume_the_receipt() {
        let receipt = receipt_fixture(
            NativeOperationOutcome::Succeeded,
            NativePostconditionEvidence {
                evidence_kind: "native_result",
                operation_succeeded: true,
                verified: true,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("mail_read_ok".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut ledger = ContinuationReceiptLedger::default();
        ledger.register(&receipt);
        let parent = receipt_parent_context();

        assert_eq!(ledger.validate(&receipt.receipt_id, &parent), Ok(()));
        assert_eq!(ledger.validate(&receipt.receipt_id, &parent), Ok(()));
        assert!(ledger.consume(&receipt.receipt_id, &parent).is_ok());
        assert_eq!(
            ledger.validate(&receipt.receipt_id, &parent),
            Err(NativeReceiptConsumptionError::Replayed)
        );
    }

    #[test]
    fn continuation_authority_rejects_tamper_and_cross_turn_use() {
        let receipt = receipt_fixture(
            NativeOperationOutcome::Unmet,
            NativePostconditionEvidence {
                evidence_kind: "native_result",
                operation_succeeded: false,
                verified: false,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("calendar_permission_denied".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut ledger = ContinuationReceiptLedger::default();
        ledger.register(&receipt);

        assert_eq!(
            ledger.consume(
                &format!("{}-tampered", receipt.receipt_id),
                &receipt_parent_context()
            ),
            Err(NativeReceiptConsumptionError::InvalidReceipt)
        );
        assert_eq!(
            ledger.consume(
                &format!(" {}", receipt.receipt_id),
                &receipt_parent_context()
            ),
            Err(NativeReceiptConsumptionError::InvalidReceipt)
        );
        let mut wrong_turn = receipt_parent_context();
        wrong_turn.turn_id = "different-turn".to_string();
        assert_eq!(
            ledger.consume(&receipt.receipt_id, &wrong_turn),
            Err(NativeReceiptConsumptionError::ContextMismatch)
        );

        let consumed = ledger
            .consume(&receipt.receipt_id, &receipt_parent_context())
            .unwrap();
        assert_eq!(consumed.outcome, NativeOperationOutcome::Unmet);
        assert!(!consumed.verified_success);
    }

    #[test]
    fn continuation_authority_is_bounded_and_rejects_workflow_receipts() {
        let mut workflow_receipt = receipt_fixture(
            NativeOperationOutcome::Succeeded,
            NativePostconditionEvidence {
                evidence_kind: "native_result",
                operation_succeeded: true,
                verified: true,
                bounded_count: None,
                truncated: None,
                native_result_code: Some("ok".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        workflow_receipt.origin =
            NativeOperationOrigin::WorkflowExecution(WorkflowExecutionOrigin {
                execution_id: "execution".to_string(),
                workflow_id: "workflow".to_string(),
                workflow_version: 1,
                node_id: "node".to_string(),
                approval_node_id: None,
                approval_binding_sha256: None,
            });
        let mut workflow_ledger = ContinuationReceiptLedger::default();
        workflow_ledger.register(&workflow_receipt);
        assert_eq!(
            workflow_ledger.consume(&workflow_receipt.receipt_id, &receipt_parent_context()),
            Err(NativeReceiptConsumptionError::WorkflowReceipt)
        );

        let base = receipt_fixture(
            NativeOperationOutcome::Succeeded,
            NativePostconditionEvidence {
                evidence_kind: "bounded_native_collection",
                operation_succeeded: true,
                verified: true,
                bounded_count: Some(0),
                truncated: Some(false),
                native_result_code: Some("ok".to_string()),
                durable_operation_binding: None,
                capture_proof: None,
            },
        );
        let mut bounded = ContinuationReceiptLedger::default();
        for index in 0..=MAX_CONTINUATION_RECEIPTS {
            let mut receipt = base.clone();
            receipt.receipt_id = format!("apple-operation-bounded-{index}");
            bounded.register(&receipt);
        }
        assert_eq!(bounded.entries.len(), MAX_CONTINUATION_RECEIPTS);
        assert_eq!(
            bounded.consume("apple-operation-bounded-0", &receipt_parent_context()),
            Err(NativeReceiptConsumptionError::InvalidReceipt)
        );
        assert!(bounded
            .consume(
                &format!("apple-operation-bounded-{MAX_CONTINUATION_RECEIPTS}"),
                &receipt_parent_context()
            )
            .is_ok());
    }

    #[test]
    fn mcp_collection_requires_a_real_structured_collection() {
        let result = McpToolCallResult {
            content: Vec::new(),
            structured_content: Some(serde_json::json!({"notes": []})),
            is_error: false,
            meta: None,
            raw: None,
        };
        let evidence = evidence_from_mcp_result(NativeActionClass::Read, &result);
        assert!(evidence.verified);
        assert_eq!(evidence.bounded_count, Some(0));
        assert!(evidence.durable_operation_binding.is_none());
    }

    #[test]
    fn exactly_once_evidence_requires_a_durable_native_effect_id() {
        let write = McpToolCallResult {
            content: Vec::new(),
            structured_content: Some(serde_json::json!({
                "id": "native-note-id-123",
                "verified": true
            })),
            is_error: false,
            meta: None,
            raw: None,
        };
        let first = evidence_from_mcp_result(NativeActionClass::Write, &write);
        let second = evidence_from_mcp_result(NativeActionClass::Write, &write);
        assert!(first.verified);
        assert_eq!(
            first.durable_operation_binding,
            second.durable_operation_binding
        );
        assert_eq!(
            first.durable_operation_binding.as_deref().map(str::len),
            Some(64)
        );

        let unverified = McpToolCallResult {
            content: Vec::new(),
            structured_content: Some(serde_json::json!({"verified": true})),
            is_error: false,
            meta: None,
            raw: None,
        };
        assert!(
            evidence_from_mcp_result(NativeActionClass::Write, &unverified)
                .durable_operation_binding
                .is_none()
        );
    }

    #[test]
    fn receipt_ids_safely_normalize_short_or_non_ascii_bindings() {
        for binding in ["", "a", "é", "short native id"] {
            let suffix = receipt_id_suffix(binding);
            assert_eq!(suffix.len(), 16);
            assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn workflow_origin_requires_a_persisted_exact_approval() {
        let path = std::env::temp_dir().join(format!(
            "oomu-native-workflow-origin-{}-{}.db",
            std::process::id(),
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let persistence = crate::db::PersistenceEngine::initialize_for_integration_test(path)
            .expect("isolated workflow origin database");
        let connection = persistence.open_connection().unwrap();
        connection
            .execute(
                "INSERT INTO workflow_blueprints (workflow_id,version,name,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES ('workflow-native',1,'Native workflow','{}',1,1,1)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO execution_instances (id,workflow_id,workflow_version,status,active_node_id,created_at_ms,updated_at_ms) VALUES ('workflow-execution','workflow-native',1,'Running','notify-node',1,1)",
                [],
            )
            .unwrap();
        drop(connection);
        let arguments = serde_json::json!({"body_text":"Ready"});
        assert!(accepted_workflow_origin(
            &persistence,
            "workflow-execution",
            "macos_applescript",
            "trigger_system_notification",
            &arguments,
            true,
            true,
        )
        .is_none());

        let approval_material = serde_json::json!({
            "arguments": arguments,
            "approvalBinding": Value::Null,
        });
        persistence
            .record_workflow_approval(
                "approval-token",
                "workflow-execution",
                "notify-node",
                "trigger_system_notification",
                &approval_material,
                "approve",
            )
            .unwrap();
        let origin = accepted_workflow_origin(
            &persistence,
            "workflow-execution",
            "macos_applescript",
            "trigger_system_notification",
            approval_material.get("arguments").unwrap(),
            true,
            true,
        )
        .expect("persisted exact approval binds workflow operation");
        assert_eq!(origin.execution_id, "workflow-execution");
        assert_eq!(origin.node_id, "notify-node");
        assert_eq!(origin.approval_node_id.as_deref(), Some("notify-node"));
        assert_eq!(
            origin.approval_binding_sha256.as_deref().map(str::len),
            Some(64)
        );
        let encoded =
            serde_json::to_value(NativeOperationOrigin::WorkflowExecution(origin)).unwrap();
        assert_eq!(
            encoded.get("kind").and_then(Value::as_str),
            Some("workflow_execution")
        );
        assert!(encoded.get("turnId").is_none());
    }
}
