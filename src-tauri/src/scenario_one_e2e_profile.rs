//! Non-interactive security profile for the opt-in Scenario 1 debug driver.
//!
//! Release builds never activate this module. Debug activation requires both
//! the exact driver flag and a disposable, tightly named temporary data root.
//! The profile uses a fresh process-memory cryptographic root; no plaintext
//! secret is persisted and no production Keychain item is read or modified.

#[cfg(debug_assertions)]
use rand_core::{OsRng, RngCore};
#[cfg(debug_assertions)]
use sha2::{Digest, Sha256};
#[cfg(debug_assertions)]
use std::collections::HashSet;
#[cfg(all(debug_assertions, unix))]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[cfg(debug_assertions)]
use std::sync::Mutex;
#[cfg(debug_assertions)]
use std::{fs, path::Component, sync::OnceLock};
#[cfg(debug_assertions)]
use zeroize::Zeroizing;

pub(crate) const ENABLE_ENV: &str = "OOMU_SCENARIO_ONE_E2E";
#[cfg(debug_assertions)]
pub(crate) const LOCAL_MODEL_ID: &str = "gemma-4-E4B-it-qat-q4_0-gguf";
#[cfg(debug_assertions)]
const ROOT_PREFIX: &str = "oomu-scenario-one-e2e-";
#[cfg(debug_assertions)]
const CALENDAR_NAME: &str = "OOMU Test";
#[cfg(debug_assertions)]
const ACCEPTANCE_CONTRACT_JSON: &str =
    include_str!("../tests/fixtures/scenario_one_acceptance_contract.json");

#[cfg(debug_assertions)]
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioOneAcceptanceContract {
    input_directory: String,
    input_paths: Vec<String>,
    output_directory: String,
    output_paths: Vec<String>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
pub(crate) struct PlanAuthorityProbe<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub generation_token: &'a str,
    pub root_turn_id: &'a str,
    pub created_at_ms: u64,
    pub automated_web_grounding_enabled: bool,
    pub model_id: &'a str,
    pub principal_approved: bool,
    pub authority_proof_absent: bool,
    pub trusted_automatic_execution: bool,
    pub exact_plan_contract: bool,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
pub(crate) struct NativeAuthorityProbe<'a> {
    pub session_id: &'a str,
    pub operation_classes: &'a [String],
    pub scopes: &'a [String],
    pub max_steps: usize,
    pub persistence: &'a str,
    pub locale: Option<&'a str>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
pub(crate) struct NativeApprovalProbe<'a> {
    pub approval_token: &'a str,
    pub session_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub generation_token: Option<&'a str>,
    pub action_type: &'a str,
    pub action_label: &'a str,
    pub target_path: Option<&'a str>,
    pub principal: Option<&'a str>,
    pub risk_tier: &'a str,
    pub reason: &'a str,
    pub estimated_token_costs: Option<usize>,
    pub requested_at_ms: u64,
    pub preview: &'a str,
    pub semantic_summary: &'a str,
    pub semantic_detail: &'a str,
    pub approval_tier: &'a str,
    pub approval_mode: &'a str,
    pub diff_preview_present: bool,
    pub scope_trust_available: bool,
    pub scope_trust_prefix: Option<&'a str>,
    pub scope_trust_duration_ms: u64,
    pub project_id: Option<&'a str>,
    pub task_run_id: Option<&'a str>,
    pub action_class: &'a str,
    pub argument_class: &'a str,
    pub canonical_resource: Option<&'a str>,
    pub mandatory_reconfirm: bool,
    pub approval_scope_kinds: &'a [String],
    pub research_policy_matches: bool,
    pub mail_payload_matches: bool,
    pub calendar_argument_class_matches: bool,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
pub(crate) struct NativeApprovalAutomation {
    scope: String,
    operation_key: String,
    action_type: String,
}

#[cfg(not(debug_assertions))]
#[derive(Clone, Debug)]
pub(crate) struct NativeApprovalAutomation {
    scope: String,
}

impl NativeApprovalAutomation {
    pub(crate) fn scope(&self) -> &str {
        &self.scope
    }
}

#[cfg(debug_assertions)]
fn acceptance_contract() -> &'static ScenarioOneAcceptanceContract {
    static CONTRACT: OnceLock<ScenarioOneAcceptanceContract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        let contract: ScenarioOneAcceptanceContract =
            serde_json::from_str(ACCEPTANCE_CONTRACT_JSON)
                .expect("Scenario 1 acceptance fixture must be valid JSON");
        assert!(
            contract.input_paths.len() == 2
                && contract.output_paths.len() == 4
                && Path::new(&contract.input_directory).is_absolute()
                && Path::new(&contract.output_directory).is_absolute()
                && contract
                    .input_paths
                    .iter()
                    .all(|path| Path::new(path).parent()
                        == Some(Path::new(&contract.input_directory)))
                && contract
                    .output_paths
                    .iter()
                    .all(|path| Path::new(path).parent()
                        == Some(Path::new(&contract.output_directory))),
            "Scenario 1 acceptance fixture must remain narrowly rooted"
        );
        contract
    })
}

#[cfg(debug_assertions)]
pub(crate) fn input_directory() -> &'static str {
    &acceptance_contract().input_directory
}

#[cfg(debug_assertions)]
pub(crate) fn input_paths() -> &'static [String] {
    &acceptance_contract().input_paths
}

#[cfg(debug_assertions)]
pub(crate) fn output_directory() -> &'static str {
    &acceptance_contract().output_directory
}

#[cfg(debug_assertions)]
pub(crate) fn output_paths() -> &'static [String] {
    &acceptance_contract().output_paths
}

#[cfg(debug_assertions)]
static ARMED_PLAN_AUTHORITY_SESSIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[cfg(debug_assertions)]
#[derive(Default)]
struct NativeApprovalAutomationState {
    pending: HashSet<String>,
    completed: HashSet<String>,
}

#[cfg(debug_assertions)]
static NATIVE_APPROVAL_AUTOMATION_STATE: OnceLock<Mutex<NativeApprovalAutomationState>> =
    OnceLock::new();

pub(crate) fn enabled() -> bool {
    cfg!(debug_assertions)
        && std::env::var_os(ENABLE_ENV).as_deref() == Some(std::ffi::OsStr::new("1"))
}

pub(crate) fn validate_activation(
    _app_data_root_env: &str,
    _isolated_root: Option<&Path>,
) -> Result<(), String> {
    if !enabled() {
        return Ok(());
    }

    #[cfg(debug_assertions)]
    {
        let root = _isolated_root.ok_or_else(|| {
            format!(
                "{ENABLE_ENV}=1 requires {_app_data_root_env} to name an isolated temporary directory."
            )
        })?;
        validate_isolated_root(root)?;
        fs::create_dir_all(root).map_err(|error| {
            format!("Scenario 1 isolated profile could not be created: {error}")
        })?;
        #[cfg(unix)]
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!("Scenario 1 isolated profile permissions could not be secured: {error}")
        })?;
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_PROFILE status=enabled storage=isolated_encrypted key_material=process_memory"
        );
    }
    Ok(())
}

#[cfg(all(debug_assertions, not(test)))]
pub(crate) fn database_secret() -> Option<Zeroizing<String>> {
    enabled().then(|| derive_secret("database"))
}

#[cfg(all(not(debug_assertions), not(test)))]
pub(crate) fn database_secret() -> Option<String> {
    None
}

#[cfg(debug_assertions)]
pub(crate) fn identity_passphrase() -> Option<Zeroizing<String>> {
    enabled().then(|| {
        Zeroizing::new(format!(
            "OOMU Scenario 1 identity {}",
            derive_secret("identity").as_str()
        ))
    })
}

#[cfg(not(debug_assertions))]
pub(crate) fn identity_passphrase() -> Option<String> {
    None
}

#[cfg(debug_assertions)]
pub(crate) fn vault(app_data_root: &Path) -> Option<std::path::PathBuf> {
    enabled().then(|| app_data_root.join(".oomu").join("vault"))
}

#[cfg(debug_assertions)]
pub(crate) fn arm_exact_plan_authority(request: &PlanAuthorityProbe<'_>) -> bool {
    if !enabled() {
        return false;
    }
    let immutable_origin = [
        request.session_id,
        request.turn_id,
        request.generation_token,
        request.root_turn_id,
    ]
    .into_iter()
    .all(|value| !value.trim().is_empty());
    let exact = immutable_origin
        && request.created_at_ms > 0
        && request.automated_web_grounding_enabled
        && request.model_id == LOCAL_MODEL_ID
        && request.principal_approved
        && request.authority_proof_absent
        && !request.trusted_automatic_execution
        && request.exact_plan_contract;
    if !exact {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=plan_authority status=human_required reason=plan_contract"
        );
        return false;
    }
    let armed = ARMED_PLAN_AUTHORITY_SESSIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .ok()
        .is_some_and(|mut sessions| sessions.insert(request.session_id.to_string()));
    if armed {
        eprintln!("OOMU_SCENARIO_ONE_E2E_TRACE stage=plan_authority status=armed_exact_session");
    }
    armed
}

#[cfg(debug_assertions)]
pub(crate) fn automated_native_authority_persistence(
    request: &NativeAuthorityProbe<'_>,
) -> Option<String> {
    if !enabled() || !exact_native_authority_contract(request) {
        return None;
    }
    let consumed = ARMED_PLAN_AUTHORITY_SESSIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .ok()
        .is_some_and(|mut sessions| sessions.remove(request.session_id.trim()));
    consumed.then(|| "one_time".to_string())
}

#[cfg(debug_assertions)]
fn exact_native_authority_contract(request: &NativeAuthorityProbe<'_>) -> bool {
    !request.session_id.trim().is_empty()
        && request.operation_classes == ["registered_task_tool".to_string()]
        && request.scopes == [format!("actuation-session:{}", request.session_id.trim())]
        && request.max_steps == 3
        && request.persistence == "session_gated"
        && request.locale.as_deref() == Some("en-US")
}

/// Selects a real native prompt button for the exact opt-in Scenario 1 run.
///
/// The caller still renders `NSAlert`, activates one of its actual buttons, and
/// passes the resulting response through the normal frozen-decision and scope
/// machinery. Anything outside the exact fixture, destination, Calendar event,
/// or unsent Mail draft remains blocked on a human decision.
#[cfg(debug_assertions)]
pub(crate) fn automated_native_approval(
    request: &NativeApprovalProbe<'_>,
) -> Option<NativeApprovalAutomation> {
    if !enabled() {
        return None;
    }
    let action_type = request
        .action_type
        .trim()
        .replace('-', "_")
        .to_ascii_lowercase();
    if !exact_request_metadata(request) {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=native_shield status=human_required reason=request_metadata action={action_type}"
        );
        return None;
    }
    let Some((scope, operation_class)) = scenario_native_scope_contract(request) else {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=native_shield status=human_required reason=scope_contract action={action_type}"
        );
        return None;
    };
    let operation_key = format!("{operation_class}:{}", request.approval_token);
    if !reserve_native_operation(&operation_key) {
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_TRACE stage=native_shield status=human_required reason=operation_already_reserved_or_completed action={action_type}"
        );
        return None;
    }
    Some(NativeApprovalAutomation {
        scope,
        operation_key,
        action_type,
    })
}

/// Commits a debug-driver approval reservation only after AppKit returns the
/// exact approved button. A prompt failure, close, denial, or mismatched scope
/// releases the reservation so the genuine workflow can retry.
#[cfg(debug_assertions)]
pub(crate) fn finish_automated_native_approval(
    automation: &NativeApprovalAutomation,
    approved: bool,
) {
    finish_native_operation(&automation.operation_key, approved);
    eprintln!(
        "OOMU_SCENARIO_ONE_E2E_TRACE stage=native_shield status={} action={}",
        if approved { "committed" } else { "released" },
        automation.action_type
    );
}

#[cfg(debug_assertions)]
fn scenario_native_scope_contract(request: &NativeApprovalProbe<'_>) -> Option<(String, String)> {
    let action_type = request
        .action_type
        .trim()
        .replace('-', "_")
        .to_ascii_lowercase();
    let target_path = request.target_path.map(str::trim);
    let exact_read = match action_type.as_str() {
        "file_list" => target_path == Some(input_directory()),
        "file_read" => {
            target_path.is_some_and(|path| input_paths().iter().any(|expected| expected == path))
        }
        _ => false,
    };
    if exact_read {
        return preferred_permitted_scope(request.approval_scope_kinds, "app_session").map(
            |scope| {
                (
                    scope,
                    format!("{action_type}:{}", target_path.unwrap_or_default()),
                )
            },
        );
    }

    let value = serde_json::from_str::<serde_json::Value>(request.preview).ok()?;
    let exact_action = match action_type.as_str() {
        "create_decision_pack" => {
            exact_decision_pack_preview(&value, target_path, request.research_policy_matches)
        }
        "create_system_calendar" => {
            target_path.is_none() && exact_calendar_creation_preview(&value)
        }
        "create_conflict_free_calendar_event" => exact_calendar_preview(&value),
        "draft_decision_pack_email" => request.mail_payload_matches,
        _ => false,
    };
    exact_action
        .then(|| preferred_permitted_scope(request.approval_scope_kinds, "once"))?
        .map(|scope| (scope, action_type))
}

#[cfg(debug_assertions)]
fn exact_request_metadata(request: &NativeApprovalProbe<'_>) -> bool {
    let action_type = request
        .action_type
        .trim()
        .replace('-', "_")
        .to_ascii_lowercase();
    if action_type == "create_system_calendar" {
        return exact_calendar_creation_request_metadata(request);
    }
    let immutable_chat_origin = [
        request.session_id,
        request.turn_id,
        request.generation_token,
    ]
    .into_iter()
    .all(|value| value.is_some_and(|value| !value.trim().is_empty()));
    let valid_token = request.approval_token.len() == 48
        && request
            .approval_token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    let complete_copy = !request.action_label.trim().is_empty()
        && !request.reason.trim().is_empty()
        && !request.semantic_summary.trim().is_empty()
        && !request.semantic_detail.trim().is_empty()
        && request
            .principal
            .is_some_and(|principal| !principal.trim().is_empty())
        && !request.argument_class.trim().is_empty()
        && request.estimated_token_costs.is_some_and(|cost| cost > 0)
        && request.requested_at_ms > 0
        && !request.diff_preview_present;
    if !immutable_chat_origin || !valid_token || !complete_copy {
        return false;
    }

    match action_type.as_str() {
        "file_list" | "file_read" => {
            let target = request.target_path.map(str::trim);
            let exact_target = if action_type == "file_list" {
                target == Some(input_directory())
            } else {
                target.is_some_and(|path| input_paths().iter().any(|expected| expected == path))
            };
            exact_target
                && request.risk_tier == "Medium Risk"
                && request.approval_tier == "visual_consent"
                && request.approval_mode == "visual"
                && request.action_class == "filesystem_read"
                && !request.mandatory_reconfirm
                && request.scope_trust_available
                && request.scope_trust_prefix == Some(input_directory())
                && request.canonical_resource == target
                && request
                    .approval_scope_kinds
                    .iter()
                    .any(|kind| kind == "once")
                && request
                    .approval_scope_kinds
                    .iter()
                    .any(|kind| kind == "app_session")
                && request
                    .approval_scope_kinds
                    .iter()
                    .all(|kind| matches!(kind.as_str(), "once" | "app_session" | "persistent"))
        }
        "create_decision_pack"
        | "create_conflict_free_calendar_event"
        | "draft_decision_pack_email" => {
            let exact_resource = match action_type.as_str() {
                "create_decision_pack" => {
                    request.target_path == Some(output_directory())
                        && request.canonical_resource == Some(output_directory())
                }
                _ => request.target_path.is_none() && request.canonical_resource.is_none(),
            };
            exact_resource
                && request.risk_tier == "High Risk"
                && request.approval_tier == "explicit_confirmation"
                && request.approval_mode == "explicit"
                && request.action_class == action_type
                && request.mandatory_reconfirm
                && !request.scope_trust_available
                && request.scope_trust_prefix.is_none()
                && request.approval_scope_kinds == ["once".to_string()]
        }
        _ => false,
    }
}

#[cfg(debug_assertions)]
fn exact_calendar_creation_request_metadata(request: &NativeApprovalProbe<'_>) -> bool {
    let immutable_origin = [
        request.session_id,
        request.turn_id,
        request.generation_token,
        request.principal,
        request.task_run_id,
    ]
    .into_iter()
    .all(|value| value.is_some_and(|value| !value.trim().is_empty()));
    let valid_token = request
        .approval_token
        .strip_prefix("approval_")
        .is_some_and(|suffix| {
            suffix.len() == 36
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
    let exact_preview = serde_json::from_str::<serde_json::Value>(request.preview)
        .is_ok_and(|value| exact_calendar_creation_preview(&value));
    immutable_origin
        && valid_token
        && exact_preview
        && request.action_type == "create_system_calendar"
        && request.action_label == "Create a calendar"
        && request.target_path.is_none()
        && request.risk_tier == "consequential"
        && request.reason == "Create the exact calendar requested by this paused task."
        && request.estimated_token_costs.is_none()
        && request.requested_at_ms > 0
        && request.semantic_summary == format!("Create the calendar “{CALENDAR_NAME}”")
        && request.semantic_detail
            == "This creates one empty calendar. The paused event step remains separate."
        && request.approval_tier == "effectful"
        && request.approval_mode == "single_exact_calendar"
        && !request.diff_preview_present
        && !request.scope_trust_available
        && request.scope_trust_prefix.is_none()
        && request.scope_trust_duration_ms == 0
        && request.project_id.is_none()
        && request.action_class == "calendar_create"
        && request.calendar_argument_class_matches
        && request.canonical_resource == Some(CALENDAR_NAME)
        && request.mandatory_reconfirm
        && request.approval_scope_kinds == ["once".to_string()]
}

#[cfg(debug_assertions)]
fn exact_calendar_creation_preview(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| object.len() == 1)
        && value
            .get("calendarName")
            .and_then(serde_json::Value::as_str)
            == Some(CALENDAR_NAME)
}

#[cfg(debug_assertions)]
fn reserve_native_operation(operation_key: &str) -> bool {
    NATIVE_APPROVAL_AUTOMATION_STATE
        .get_or_init(|| Mutex::new(NativeApprovalAutomationState::default()))
        .lock()
        .ok()
        .is_some_and(|mut state| {
            if state.pending.contains(operation_key) || state.completed.contains(operation_key) {
                return false;
            }
            state.pending.insert(operation_key.to_string())
        })
}

#[cfg(debug_assertions)]
fn finish_native_operation(operation_key: &str, approved: bool) {
    if let Ok(mut state) = NATIVE_APPROVAL_AUTOMATION_STATE
        .get_or_init(|| Mutex::new(NativeApprovalAutomationState::default()))
        .lock()
    {
        state.pending.remove(operation_key);
        if approved {
            state.completed.insert(operation_key.to_string());
        }
    }
}

#[cfg(debug_assertions)]
fn preferred_permitted_scope(permitted_scope_kinds: &[String], preferred: &str) -> Option<String> {
    [preferred, "once"]
        .into_iter()
        .find(|candidate| permitted_scope_kinds.iter().any(|kind| kind == candidate))
        .map(str::to_string)
}

#[cfg(debug_assertions)]
fn exact_decision_pack_preview(
    value: &serde_json::Value,
    target_path: Option<&str>,
    research_policy_matches: bool,
) -> bool {
    let exact_fields = value.as_object().is_some_and(|object| {
        object.len() == 7
            && [
                "action",
                "inputPaths",
                "researchPolicy",
                "outputDirectory",
                "outputs",
                "willOverwrite",
                "calendarOrMailIncluded",
            ]
            .iter()
            .all(|field| object.contains_key(*field))
    });
    let exact_inputs = value
        .get("inputPaths")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|paths| {
            paths.len() == input_paths().len()
                && input_paths().iter().all(|expected| {
                    paths
                        .iter()
                        .any(|path| path.as_str() == Some(expected.as_str()))
                })
        });
    let exact_outputs = value
        .get("outputs")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|outputs| {
            outputs.len() == 4
                && outputs.get("workbook").and_then(serde_json::Value::as_str)
                    == Some("supplier_decision.xlsx")
                && outputs
                    .get("presentation")
                    .and_then(serde_json::Value::as_str)
                    == Some("supplier_decision.pptx")
                && outputs.get("pdf").and_then(serde_json::Value::as_str)
                    == Some("supplier_decision.pdf")
                && outputs.get("sources").and_then(serde_json::Value::as_str) == Some("sources.md")
        });
    exact_fields
        && value.get("action").and_then(serde_json::Value::as_str) == Some("create_decision_pack")
        && target_path == Some(output_directory())
        && value
            .get("outputDirectory")
            .and_then(serde_json::Value::as_str)
            == Some(output_directory())
        && value
            .get("willOverwrite")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        && value
            .get("calendarOrMailIncluded")
            .and_then(serde_json::Value::as_bool)
            == Some(false)
        && exact_inputs
        && exact_outputs
        && research_policy_matches
}

#[cfg(debug_assertions)]
fn exact_calendar_preview(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == 9
            && [
                "calendarName",
                "title",
                "day",
                "windowStartLocal",
                "windowEndLocal",
                "durationMinutes",
                "location",
                "notes",
                "availability",
            ]
            .iter()
            .all(|field| object.contains_key(*field))
    }) && value
        .get("calendarName")
        .and_then(serde_json::Value::as_str)
        == Some(CALENDAR_NAME)
        && value.get("title").and_then(serde_json::Value::as_str)
            == Some("Supplier Decision Review")
        && value.get("day").and_then(serde_json::Value::as_str) == Some("next_weekday")
        && value
            .get("windowStartLocal")
            .and_then(serde_json::Value::as_str)
            == Some("13:00")
        && value
            .get("windowEndLocal")
            .and_then(serde_json::Value::as_str)
            == Some("16:00")
        && value
            .get("durationMinutes")
            .and_then(serde_json::Value::as_u64)
            == Some(30)
        && value.get("location").and_then(serde_json::Value::as_str) == Some("")
        && value.get("notes").and_then(serde_json::Value::as_str)
            == Some(
                "Review the verified supplier decision pack and its evidence-bound recommendation.",
            )
        && value
            .get("availability")
            .and_then(serde_json::Value::as_str)
            == Some("tentative")
}

#[cfg(not(debug_assertions))]
pub(crate) fn vault(_app_data_root: &Path) -> Option<std::path::PathBuf> {
    None
}

#[cfg(debug_assertions)]
fn derive_secret(label: &str) -> Zeroizing<String> {
    let root = process_secret();
    let digest =
        Sha256::digest(format!("oomu.scenario-one.e2e.v1:{label}:{}", root.as_str()).as_bytes());
    Zeroizing::new(hex::encode(digest))
}

#[cfg(debug_assertions)]
fn process_secret() -> &'static Zeroizing<String> {
    static SECRET: OnceLock<Zeroizing<String>> = OnceLock::new();
    SECRET.get_or_init(|| {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let encoded = hex::encode(bytes);
        bytes.fill(0);
        Zeroizing::new(encoded)
    })
}

#[cfg(debug_assertions)]
fn validate_isolated_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err("Scenario 1 isolated profile root must be an absolute normalized path.".into());
    }
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with(ROOT_PREFIX) && name.len() > ROOT_PREFIX.len())
        .ok_or_else(|| {
            format!("Scenario 1 isolated profile root must begin with {ROOT_PREFIX}.")
        })?;
    if name.contains(std::path::MAIN_SEPARATOR) {
        return Err("Scenario 1 isolated profile name is invalid.".into());
    }

    let parent = root
        .parent()
        .ok_or_else(|| "Scenario 1 isolated profile root has no parent.".to_string())?;
    let parent = fs::canonicalize(parent).map_err(|error| {
        format!("Scenario 1 isolated profile parent could not be verified: {error}")
    })?;
    let system_tmp = fs::canonicalize(std::env::temp_dir()).ok();
    let private_tmp = fs::canonicalize("/private/tmp").ok();
    if system_tmp.as_ref() != Some(&parent) && private_tmp.as_ref() != Some(&parent) {
        return Err(
            "Scenario 1 isolated profile root must be a direct child of a temporary directory."
                .into(),
        );
    }
    if let Ok(metadata) = fs::symlink_metadata(root) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(
                "Scenario 1 isolated profile root must be a real directory, not a link or file."
                    .into(),
            );
        }
    }
    Ok(())
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    const READ_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CALENDAR_TOKEN: &str = "approval_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CALENDAR_PREVIEW: &str = r#"{"calendarName":"OOMU Test"}"#;

    fn read_probe<'a>(scopes: &'a [String]) -> NativeApprovalProbe<'a> {
        NativeApprovalProbe {
            approval_token: READ_TOKEN,
            session_id: Some("session-1"),
            turn_id: Some("turn-1"),
            generation_token: Some("generation-1"),
            action_type: "file_list",
            action_label: "View a local folder",
            target_path: Some(input_directory()),
            principal: Some("agent-1"),
            risk_tier: "Medium Risk",
            reason: "This folder is outside the folders OOMU can currently use.",
            estimated_token_costs: Some(2),
            requested_at_ms: 1,
            preview: input_directory(),
            semantic_summary: "Use mock_data to finish this request.",
            semantic_detail: "Nothing will be changed.",
            approval_tier: "visual_consent",
            approval_mode: "visual",
            diff_preview_present: false,
            scope_trust_available: true,
            scope_trust_prefix: Some(input_directory()),
            scope_trust_duration_ms: 1,
            project_id: None,
            task_run_id: None,
            action_class: "filesystem_read",
            argument_class: "filesystem_read",
            canonical_resource: Some(input_directory()),
            mandatory_reconfirm: false,
            approval_scope_kinds: scopes,
            research_policy_matches: false,
            mail_payload_matches: false,
            calendar_argument_class_matches: false,
        }
    }

    fn calendar_probe<'a>(scopes: &'a [String]) -> NativeApprovalProbe<'a> {
        NativeApprovalProbe {
            approval_token: CALENDAR_TOKEN,
            session_id: Some("session-1"),
            turn_id: Some("turn-1"),
            generation_token: Some("generation-1"),
            action_type: "create_system_calendar",
            action_label: "Create a calendar",
            target_path: None,
            principal: Some("agent-1"),
            risk_tier: "consequential",
            reason: "Create the exact calendar requested by this paused task.",
            estimated_token_costs: None,
            requested_at_ms: 1,
            preview: CALENDAR_PREVIEW,
            semantic_summary: "Create the calendar “OOMU Test”",
            semantic_detail:
                "This creates one empty calendar. The paused event step remains separate.",
            approval_tier: "effectful",
            approval_mode: "single_exact_calendar",
            diff_preview_present: false,
            scope_trust_available: false,
            scope_trust_prefix: None,
            scope_trust_duration_ms: 0,
            project_id: None,
            task_run_id: Some("execution-1"),
            action_class: "calendar_create",
            argument_class: "calendar_create:exact",
            canonical_resource: Some(CALENDAR_NAME),
            mandatory_reconfirm: true,
            approval_scope_kinds: scopes,
            research_policy_matches: false,
            mail_payload_matches: false,
            calendar_argument_class_matches: true,
        }
    }

    #[test]
    fn isolated_profile_root_is_narrow_and_temporary() {
        let accepted = std::env::temp_dir().join("oomu-scenario-one-e2e-unit");
        assert!(validate_isolated_root(&accepted).is_ok());
        assert!(validate_isolated_root(Path::new("relative-profile")).is_err());
        assert!(validate_isolated_root(Path::new("/private/tmp/other-profile")).is_err());
        assert!(validate_isolated_root(Path::new("/private")).is_err());
    }

    #[test]
    fn native_prompt_automation_is_exact_and_least_privileged() {
        let read_scopes = vec![
            "once".to_string(),
            "app_session".to_string(),
            "persistent".to_string(),
        ];
        let read = read_probe(&read_scopes);
        assert_eq!(
            scenario_native_scope_contract(&read).map(|(scope, _)| scope),
            Some("app_session".to_string())
        );
        let mut escaped_read = read;
        escaped_read.target_path = Some("/Users/example");
        escaped_read.preview = "/Users/example";
        assert!(scenario_native_scope_contract(&escaped_read).is_none());

        let preview = serde_json::json!({
            "action": "create_decision_pack",
            "inputPaths": input_paths(),
            "researchPolicy": { "callerValidated": true },
            "outputDirectory": output_directory(),
            "outputs": {
                "workbook": "supplier_decision.xlsx",
                "presentation": "supplier_decision.pptx",
                "pdf": "supplier_decision.pdf",
                "sources": "sources.md"
            },
            "willOverwrite": false,
            "calendarOrMailIncluded": false
        })
        .to_string();
        let once = vec!["once".to_string()];
        let mut decision_pack = read_probe(&once);
        decision_pack.action_type = "create_decision_pack";
        decision_pack.target_path = Some(output_directory());
        decision_pack.preview = &preview;
        decision_pack.research_policy_matches = true;
        assert_eq!(
            scenario_native_scope_contract(&decision_pack).map(|(scope, _)| scope),
            Some("once".to_string())
        );

        let mut escaped = serde_json::from_str::<serde_json::Value>(&preview).unwrap();
        escaped["outputDirectory"] = serde_json::json!("/private/tmp/oomu-scenario-one-e2e-other");
        let escaped = escaped.to_string();
        decision_pack.preview = &escaped;
        assert!(scenario_native_scope_contract(&decision_pack).is_none());
    }

    #[test]
    fn missing_calendar_automation_accepts_only_the_exact_recovery_approval() {
        let once = vec!["once".to_string()];
        let mut request = calendar_probe(&once);
        assert!(exact_request_metadata(&request));
        assert_eq!(
            scenario_native_scope_contract(&request),
            Some(("once".to_string(), "create_system_calendar".to_string()))
        );

        request.preview = r#"{"calendarName":"Personal"}"#;
        assert!(!exact_request_metadata(&request));
        assert!(scenario_native_scope_contract(&request).is_none());
        request.preview = CALENDAR_PREVIEW;
        request.canonical_resource = Some("Personal");
        assert!(!exact_request_metadata(&request));
        request.canonical_resource = Some(CALENDAR_NAME);
        let expanded = vec!["once".to_string(), "app_session".to_string()];
        request.approval_scope_kinds = &expanded;
        assert!(!exact_request_metadata(&request));
    }

    #[test]
    fn native_prompt_automation_requires_complete_immutable_request_metadata() {
        let scopes = vec![
            "once".to_string(),
            "app_session".to_string(),
            "persistent".to_string(),
        ];
        let mut request = read_probe(&scopes);
        assert!(exact_request_metadata(&request));
        request.generation_token = None;
        assert!(!exact_request_metadata(&request));
        request.generation_token = Some("generation-1");
        request.approval_mode = "background";
        assert!(!exact_request_metadata(&request));
    }

    #[test]
    fn native_prompt_automation_commits_only_after_real_approval() {
        let first = "test_release_after_prompt_failure";
        assert!(reserve_native_operation(first));
        assert!(!reserve_native_operation(first));
        finish_native_operation(first, false);
        assert!(reserve_native_operation(first));
        finish_native_operation(first, true);
        assert!(!reserve_native_operation(first));

        let second = "test_distinct_native_operation";
        assert!(reserve_native_operation(second));
        finish_native_operation(second, true);

        let recovered = "create_conflict_free_calendar_event:fresh-approval-token";
        assert!(reserve_native_operation(recovered));
        finish_native_operation(recovered, true);
    }

    #[test]
    fn plan_authority_automation_requires_the_exact_one_time_contract() {
        let operation_classes = vec!["registered_task_tool".to_string()];
        let expected_scopes = vec!["actuation-session:session-1".to_string()];
        let wrong_scopes = vec!["actuation-session:another-session".to_string()];
        let mut request = NativeAuthorityProbe {
            session_id: "session-1",
            operation_classes: &operation_classes,
            scopes: &expected_scopes,
            max_steps: 3,
            persistence: "session_gated",
            locale: Some("en-US"),
        };
        assert!(exact_native_authority_contract(&request));
        request.max_steps = 4;
        assert!(!exact_native_authority_contract(&request));
        request.max_steps = 3;
        request.scopes = &wrong_scopes;
        assert!(!exact_native_authority_contract(&request));
        request.scopes = &expected_scopes;
        request.persistence = "global_trust";
        assert!(!exact_native_authority_contract(&request));
    }
}
