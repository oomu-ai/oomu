mod authority;
#[cfg(test)]
mod authority_tests;

use super::{adapter, api, repository, ConnectorOperationRequest, ConnectorResultSource};
use crate::{
    db::PersistenceEngine, foundation::digest::sha256_hex, p0_contracts::EvidenceClass,
    shield_gate::ShieldApprovalManager,
};
use authority::{executable_capabilities, require_planned_connector_authority};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::Manager;

const MAX_ARGUMENT_BYTES: usize = 256 * 1024;
const MAX_AGENT_RESULT_BYTES: usize = 64 * 1024;
const MAX_EVIDENCE_EXCERPT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskConnectorToolRequest {
    #[serde(alias = "connector_ref")]
    pub connector_ref: String,
    pub capability: String,
    #[serde(default)]
    pub arguments: Value,
}

impl TaskConnectorToolRequest {
    #[cfg(test)]
    pub(crate) fn new(connector_ref: String, capability: String, arguments: Value) -> Self {
        Self {
            connector_ref,
            capability,
            arguments,
        }
    }

    pub(crate) fn potentially_effectful(&self) -> bool {
        matches!(
            self.capability.as_str(),
            "draft_email"
                | "draft_calendar_event"
                | "save_personal_file"
                | "save_team_file"
                | "draft_chat_message"
        )
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskConnectorToolResult {
    pub capability: String,
    pub observed_at_ms: i64,
    pub source: ConnectorResultSource,
    pub partial: bool,
    pub result: Value,
    pub source_ref: String,
    pub evidence_ref: String,
    pub evidence_digest: String,
    pub postcondition_recorded: bool,
}

pub(crate) fn command_response(
    result: TaskConnectorToolResult,
) -> Result<crate::shield_gate::ExecuteCommandResponse, String> {
    let citation_digest = sha256_hex(result.source.citation.as_bytes());
    let message = serde_json::to_string(&json!({
        "capability":result.capability,
        "source":result.source,
        "partial":result.partial,
        "result":result.result,
        "sourceRef":result.source_ref,
        "evidenceRef":result.evidence_ref,
        "evidenceDigest":result.evidence_digest,
        "postconditionRecorded":result.postcondition_recorded,
    }))
    .map_err(|_| "connector_task_result_invalid".to_string())?;
    Ok(crate::shield_gate::ExecuteCommandResponse {
        operation: "connected_work".to_string(),
        status: crate::shield_gate::CommandStatus::Completed,
        message,
        metrics: None,
        claims: vec![format!(
            "CLAIM connector_task_evidence result_sha256={} citation_sha256={} evidence_recorded=true postcondition_recorded={}",
            result.evidence_digest, citation_digest, result.postcondition_recorded
        )],
        verified: true,
        model_used: None,
    })
}

pub(crate) async fn execute_agent_task_command(
    engine: &PersistenceEngine,
    app: Option<&tauri::AppHandle>,
    execution_id: Option<&str>,
    request: TaskConnectorToolRequest,
) -> Result<crate::shield_gate::ExecuteCommandResponse, String> {
    let execution_id =
        execution_id.ok_or_else(|| "Connected work requires an active agent Task.".to_string())?;
    let approvals = app.and_then(|app| tauri::Manager::try_state::<ShieldApprovalManager>(app));
    execute_for_agent_task(engine, app, approvals.as_deref(), execution_id, request)
        .await
        .and_then(command_response)
}

pub(crate) fn validate_task_tool_request(
    request: TaskConnectorToolRequest,
) -> Result<TaskConnectorToolRequest, String> {
    crate::p0_contracts::ConnectorId::parse(request.connector_ref.trim())?;
    if !known_capability(request.capability.trim()) {
        return Err("connector_task_capability_unsupported".to_string());
    }
    if !request.arguments.is_object() {
        return Err("connector_task_arguments_invalid".to_string());
    }
    let encoded = serde_json::to_vec(&request.arguments)
        .map_err(|_| "connector_task_arguments_invalid".to_string())?;
    if encoded.len() > MAX_ARGUMENT_BYTES {
        return Err("connector_task_arguments_too_large".to_string());
    }
    Ok(TaskConnectorToolRequest {
        connector_ref: request.connector_ref.trim().to_string(),
        capability: request.capability.trim().to_string(),
        ..request
    })
}

fn known_capability(value: &str) -> bool {
    matches!(
        value,
        "find_email"
            | "read_email"
            | "draft_email"
            | "read_calendar"
            | "draft_calendar_event"
            | "find_personal_files"
            | "read_personal_file"
            | "save_personal_file"
            | "find_team_files"
            | "read_team_file"
            | "save_team_file"
            | "find_team_site"
            | "list_chats"
            | "find_chat_messages"
            | "draft_chat_message"
    )
}

pub(crate) fn resolve_task_tool_dependencies(
    mut request: TaskConnectorToolRequest,
    prior_outputs: &[crate::shield_gate::ExecuteCommandResponse],
) -> Result<TaskConnectorToolRequest, String> {
    let parsed = prior_outputs
        .iter()
        .map(|output| {
            if !output.verified || output.operation != "connected_work" {
                return Ok(None);
            }
            serde_json::from_str::<Value>(&output.message)
                .map(Some)
                .map_err(|_| "connector_task_dependency_invalid".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut bindings = 0_usize;
    resolve_value(&mut request.arguments, &parsed, 0, &mut bindings)?;
    validate_task_tool_request(request)
}

fn resolve_value(
    value: &mut Value,
    outputs: &[Option<Value>],
    depth: usize,
    bindings: &mut usize,
) -> Result<(), String> {
    if depth > 12 {
        return Err("connector_task_dependency_too_deep".to_string());
    }
    if let Some(replacement) = dependency_value(value, outputs)? {
        *bindings += 1;
        if *bindings > 16 {
            return Err("connector_task_dependency_too_many".to_string());
        }
        *value = replacement;
        return Ok(());
    }
    match value {
        Value::Array(values) => {
            for value in values {
                resolve_value(value, outputs, depth + 1, bindings)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                resolve_value(value, outputs, depth + 1, bindings)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn dependency_value(value: &Value, outputs: &[Option<Value>]) -> Result<Option<Value>, String> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(step_value) = object.get("$fromStep") else {
        return Ok(None);
    };
    let step = step_value
        .as_u64()
        .ok_or_else(|| "connector_task_dependency_selector_invalid".to_string())?;
    let source = outputs
        .get(step as usize)
        .ok_or_else(|| "connector_task_dependency_not_prior".to_string())?
        .as_ref()
        .ok_or_else(|| "connector_task_dependency_unverified".to_string())?;
    if let Some(pointer) = object.get("pointer").and_then(Value::as_str) {
        if object.len() != 2 || !safe_pointer(pointer) {
            return Err("connector_task_dependency_pointer_invalid".to_string());
        }
        let selected = source
            .pointer(pointer)
            .ok_or_else(|| "connector_task_dependency_not_found".to_string())?;
        return bounded_scalar(selected).map(Some);
    }
    if object.len() != 4 {
        return Err("connector_task_dependency_selector_invalid".to_string());
    }
    let collection = object
        .get("collection")
        .and_then(Value::as_str)
        .filter(|pointer| safe_pointer(pointer))
        .ok_or_else(|| "connector_task_dependency_selector_invalid".to_string())?;
    let matcher = object
        .get("match")
        .and_then(Value::as_object)
        .filter(|matcher| matcher.len() == 2)
        .ok_or_else(|| "connector_task_dependency_selector_invalid".to_string())?;
    let field = safe_field(matcher.get("field"))?;
    let expected = bounded_scalar(
        matcher
            .get("equals")
            .ok_or_else(|| "connector_task_dependency_selector_invalid".to_string())?,
    )?;
    let select = safe_field(object.get("select"))?;
    let values = source
        .pointer(collection)
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 100)
        .ok_or_else(|| "connector_task_dependency_collection_invalid".to_string())?;
    let matches = values
        .iter()
        .filter(|candidate| candidate.get(field) == Some(&expected))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err("connector_task_dependency_ambiguous".to_string());
    }
    bounded_scalar(
        matches[0]
            .get(select)
            .ok_or_else(|| "connector_task_dependency_not_found".to_string())?,
    )
    .map(Some)
}

fn safe_pointer(pointer: &str) -> bool {
    pointer.starts_with("/result/")
        && pointer.len() <= 256
        && !pointer
            .split('/')
            .any(|segment| !segment.is_empty() && segment.parse::<usize>().is_ok())
}

fn safe_field(value: Option<&Value>) -> Result<&str, String> {
    value
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .ok_or_else(|| "connector_task_dependency_selector_invalid".to_string())
}

fn bounded_scalar(value: &Value) -> Result<Value, String> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(value.clone()),
        Value::String(text) if text.len() <= 8 * 1024 && !text.contains('\0') => Ok(value.clone()),
        _ => Err("connector_task_dependency_value_invalid".to_string()),
    }
}

pub(crate) fn planner_tool_context(
    engine: &PersistenceEngine,
    session_id: &str,
) -> Result<Option<String>, String> {
    let project_id: Option<String> = engine
        .open_connection()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT project_id FROM chat_sessions WHERE id=?1",
            rusqlite::params![session_id.trim()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let Some(project_id) = project_id else {
        return Ok(None);
    };
    let Ok(project_id) = crate::projects::repository::validate_user_project(engine, &project_id)
    else {
        return Ok(None);
    };
    let accounts = repository::list_accounts(engine)?;
    let project_accounts = accounts
        .into_iter()
        .filter(|account| {
            account.all_projects_enabled
                || account
                    .enabled_project_ids
                    .iter()
                    .any(|id| id == &project_id)
        })
        .collect::<Vec<_>>();
    let reconnect_needed = project_accounts.iter().any(|account| {
        !matches!(
            account.connection_state.as_str(),
            "authorized" | "reachable"
        )
    });
    let lines = project_accounts
        .into_iter()
        .filter(|account| {
            matches!(
                account.connection_state.as_str(),
                "authorized" | "reachable"
            )
        })
        .filter_map(|account| {
            let registered = adapter::for_manifest(&account.manifest_id)?;
            let capabilities = executable_capabilities(&account.capability_grants, registered);
            (!capabilities.is_empty()).then(|| {
                format!(
                    "- connectorRef={} capabilities={}",
                    account.connector_id,
                    capabilities.join(",")
                )
            })
        })
        .collect::<Vec<_>>();
    if lines.is_empty() && !reconnect_needed {
        return Ok(None);
    }
    let unavailable = reconnect_needed.then_some("\nAt least one Project-enabled account needs reconnection. Do not plan a call against it; explain that the user must reconnect it in Integrations.").unwrap_or_default();
    let instructions = "Connected Work Tool\nThe following internal account references are ready for this Project. Never repeat an account reference to the user. Use connected_work only when the requested capability is listed; pass the account reference as connector_ref, a listed friendly capability, and a bounded arguments object. Never invent an operation code or bypass Project policy, consent, or Shield approval. A later connected_work step may bind a scalar from an earlier verified step with {\"$fromStep\":0,\"pointer\":\"/result/id\"}. Never index an array. Select one exact item with {\"$fromStep\":0,\"collection\":\"/result/value\",\"match\":{\"field\":\"subject\",\"equals\":\"Exact subject\"},\"select\":\"id\"}; zero or multiple matches halt safely.";
    Ok(Some(format!(
        "{instructions}\n{}{}",
        lines.join("\n"),
        unavailable
    )))
}

pub(crate) async fn execute_for_agent_task(
    engine: &PersistenceEngine,
    app: Option<&tauri::AppHandle>,
    approvals: Option<&ShieldApprovalManager>,
    execution_id: &str,
    request: TaskConnectorToolRequest,
) -> Result<TaskConnectorToolResult, String> {
    let task = crate::tasks::require_agent_runtime_task(engine, execution_id)?;
    execute_for_task(engine, app, approvals, &task.task_run_id, request).await
}

async fn execute_for_task(
    engine: &PersistenceEngine,
    app: Option<&tauri::AppHandle>,
    approvals: Option<&ShieldApprovalManager>,
    task_run_id: &str,
    request: TaskConnectorToolRequest,
) -> Result<TaskConnectorToolResult, String> {
    let request = validate_task_tool_request(request)?;
    let task = crate::tasks::task_for_connector(engine, task_run_id)?;
    let project_id = task
        .project_id
        .clone()
        .ok_or_else(|| "connector_task_project_required".to_string())?;
    let authority = require_planned_connector_authority(
        engine,
        &request.connector_ref,
        None,
        None,
        Some(&project_id),
        &request.capability,
    )?;
    let operation = authority.operation;
    let operation_result = api::execute(
        engine,
        app,
        approvals,
        app.and_then(|handle| {
            handle
                .try_state::<crate::sovereign_identity::SovereignIdentity>()
                .map(|state| state.inner())
        }),
        ConnectorOperationRequest {
            connector_id: request.connector_ref,
            project_id,
            task_id: Some(task.task_id.clone()),
            task_run_id: Some(task.task_run_id.clone()),
            operation: operation.to_string(),
            arguments: request.arguments,
        },
    )
    .await?;
    let encoded = serde_json::to_vec(&operation_result.result)
        .map_err(|_| "connector_task_result_invalid".to_string())?;
    let evidence_digest = sha256_hex(&encoded);
    let postcondition = bounded_postcondition(&operation_result.result);
    let postcondition_recorded = postcondition.is_some();
    let result_excerpt =
        (encoded.len() <= MAX_EVIDENCE_EXCERPT_BYTES).then(|| operation_result.result.clone());
    let evidence = json!({
        "capability":request.capability,
        "connectorRefHash":sha256_hex(operation_result.connector_id.as_bytes()),
        "source":operation_result.source,
        "partial":operation_result.partial,
        "observedAtMs":operation_result.observed_at_ms,
        "resultDigest":evidence_digest,
        "resultBytes":encoded.len(),
        "resultExcerpt":result_excerpt,
        "accountBindingHash":operation_result.account_binding_hash,
        "tenantBindingHash":operation_result.tenant_binding_hash,
        "postcondition":postcondition,
    });
    let evidence_class = if postcondition_recorded {
        EvidenceClass::VerifiedPostcondition
    } else {
        EvidenceClass::ObservedResult
    };
    let event_sequence = crate::tasks::record_domain_event_with_sequence(
        engine,
        &task.task_run_id,
        "connector.tool.completed",
        evidence_class,
        evidence,
    )?;
    let source_ref = "connector.tool.completed".to_string();
    let evidence_ref = format!("task-event:{}:{event_sequence}", task.task_run_id);
    let agent_result = if encoded.len() <= MAX_AGENT_RESULT_BYTES {
        operation_result.result
    } else {
        json!({
            "bounded":true,
            "resultDigest":evidence_digest,
            "resultBytes":encoded.len(),
            "message":"The full result exceeded the conversational bound; its source and digest were recorded in the Task evidence timeline."
        })
    };
    Ok(TaskConnectorToolResult {
        capability: request.capability,
        observed_at_ms: operation_result.observed_at_ms,
        source: operation_result.source,
        partial: operation_result.partial,
        result: agent_result,
        source_ref,
        evidence_ref,
        evidence_digest,
        postcondition_recorded,
    })
}

fn bounded_postcondition(result: &Value) -> Option<Value> {
    if let Some(value) = result
        .get("mutationPostcondition")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
    {
        return Some(json!({"mutationPostcondition":value}));
    }
    if result.get("localDraft").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let mut observed = serde_json::Map::new();
    observed.insert("localDraft".to_string(), Value::Bool(true));
    for field in ["eventCreated", "invitationsSent", "posted"] {
        if let Some(value) = result.get(field).and_then(Value::as_bool) {
            observed.insert(field.to_string(), Value::Bool(value));
        }
    }
    Some(Value::Object(observed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::{CreateProjectRequest, ProjectDataPolicy};
    use rusqlite::params;

    fn connected_output(value: Value) -> crate::shield_gate::ExecuteCommandResponse {
        crate::shield_gate::ExecuteCommandResponse {
            operation: "connected_work".into(),
            status: crate::shield_gate::CommandStatus::Completed,
            message: json!({"result":value}).to_string(),
            metrics: None,
            claims: vec!["bounded".into()],
            verified: true,
            model_used: None,
        }
    }

    #[test]
    fn dependency_binding_requires_a_unique_verified_scalar() {
        let output = connected_output(json!({"value":[
            {"id":"mail-1","subject":"Exact subject"},
            {"id":"mail-2","subject":"Other"}
        ]}));
        let request = TaskConnectorToolRequest::new(
            "connector_11111111-1111-4111-8111-111111111111".into(),
            "read_email".into(),
            json!({"messageId":{"$fromStep":0,"collection":"/result/value","match":{"field":"subject","equals":"Exact subject"},"select":"id"}}),
        );
        let resolved =
            resolve_task_tool_dependencies(request, std::slice::from_ref(&output)).unwrap();
        assert_eq!(resolved.arguments["messageId"], "mail-1");
        let ambiguous = TaskConnectorToolRequest::new(
            "connector_11111111-1111-4111-8111-111111111111".into(),
            "read_email".into(),
            json!({"messageId":{"$fromStep":0,"collection":"/result/value","match":{"field":"subject","equals":"Exact subject"},"select":"id"}}),
        );
        let duplicate = connected_output(json!({"value":[
            {"id":"mail-1","subject":"Exact subject"},
            {"id":"mail-3","subject":"Exact subject"}
        ]}));
        assert_eq!(
            resolve_task_tool_dependencies(ambiguous, &[duplicate]).unwrap_err(),
            "connector_task_dependency_ambiguous"
        );
        let indexed = TaskConnectorToolRequest::new(
            "connector_11111111-1111-4111-8111-111111111111".into(),
            "read_email".into(),
            json!({"messageId":{"$fromStep":0,"pointer":"/result/value/0/id"}}),
        );
        assert_eq!(
            resolve_task_tool_dependencies(indexed, &[output]).unwrap_err(),
            "connector_task_dependency_pointer_invalid"
        );
        let malformed = TaskConnectorToolRequest::new(
            "connector_11111111-1111-4111-8111-111111111111".into(),
            "read_email".into(),
            json!({"messageId":{"$fromStep":"zero","pointer":"/result/id"}}),
        );
        assert_eq!(
            resolve_task_tool_dependencies(malformed, &[]).unwrap_err(),
            "connector_task_dependency_selector_invalid"
        );
    }

    #[test]
    fn model_request_cannot_carry_project_consent() {
        assert!(serde_json::from_value::<TaskConnectorToolRequest>(json!({
            "connectorRef":"connector_11111111-1111-4111-8111-111111111111",
            "capability":"find_email",
            "arguments":{"query":"quarterly"},
            "projectConsent":true
        }))
        .is_err());
    }

    #[test]
    fn degraded_project_account_is_guidance_not_a_callable_tool() {
        let root = std::env::temp_dir().join(format!(
            "oomu-connector-catalog-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "Catalog".into(),
                description: String::new(),
                data_policy: ProjectDataPolicy::AskBeforeCloud,
            },
        )
        .unwrap();
        let session = engine
            .ensure_chat_session(crate::db::CreateChatSessionRequest {
                agent_id: "agent-test".into(),
                provider_id: "local_model".into(),
                model_id: "model-test".into(),
                title: Some("Catalog".into()),
                dynamic_routing_override: None,
                workspace_id: None,
            })
            .unwrap();
        crate::projects::repository::bind_record(
            &engine,
            crate::projects::BindProjectRecordRequest {
                project_id: Some(project.project_id.clone()),
                record_kind: "chat_session".into(),
                record_id: session.id.clone(),
            },
        )
        .unwrap();
        let connector =
            repository::create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1)
                .unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='authorized' WHERE connector_id=?1",
                params![connector],
            )
            .unwrap();
        repository::set_project_binding(&engine, &connector, &project.project_id, true).unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='degraded' WHERE connector_id=?1",
                params![connector],
            )
            .unwrap();
        let context = planner_tool_context(&engine, &session.id).unwrap().unwrap();
        assert!(context.contains("needs reconnection"));
        assert!(!context.contains(&format!("connectorRef={connector}")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn task_bound_runtime_reaches_generic_adapter_and_records_bounded_evidence() {
        let root = std::env::temp_dir().join(format!(
            "oomu-connector-task-runtime-{}",
            crate::foundation::clock::unix_time_ms_i64()
        ));
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
        let project = crate::projects::repository::create(
            &engine,
            CreateProjectRequest {
                name: "Connected work test".into(),
                description: String::new(),
                data_policy: ProjectDataPolicy::LocalOnly,
            },
        )
        .unwrap();
        let task_id = crate::p0_contracts::TaskId::new().to_string();
        let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
        let now = crate::foundation::clock::unix_time_ms_i64();
        engine.open_connection().unwrap().execute("INSERT INTO task_runs (task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'agent','agent-test','running','agent',?2,'Draft a reply',?4,?4,'reconciled')", params![task_run_id,task_id,project.project_id,now]).unwrap();
        let connector =
            repository::create_account(&engine, super::super::microsoft365::MANIFEST_ID, 1)
                .unwrap();
        engine
            .open_connection()
            .unwrap()
            .execute(
                "UPDATE connector_accounts SET connection_state='authorized' WHERE connector_id=?1",
                params![connector],
            )
            .unwrap();
        repository::set_project_binding(&engine, &connector, &project.project_id, true).unwrap();
        let result = execute_for_task(
            &engine,
            None,
            None,
            &task_run_id,
            TaskConnectorToolRequest {
                connector_ref: connector,
                capability: "draft_chat_message".into(),
                arguments: json!({"chatId":"chat-7","text":"Ready for review"}),
            },
        )
        .await
        .unwrap();
        assert_eq!(result.capability, "draft_chat_message");
        assert_eq!(result.result["posted"], false);
        assert!(result.postcondition_recorded);
        assert_eq!(result.source_ref, "connector.tool.completed");
        assert_eq!(result.evidence_ref, format!("task-event:{task_run_id}:0"));
        let command = command_response(result.clone()).unwrap();
        let command_json: Value = serde_json::from_str(&command.message).unwrap();
        assert_eq!(command_json["sourceRef"], "connector.tool.completed");
        assert_eq!(command_json["evidenceRef"], result.evidence_ref);
        assert!(command.verified);
        let event: String = engine.open_connection().unwrap().query_row("SELECT event_json FROM task_events WHERE task_run_id=?1 AND event_json LIKE '%connector.tool.completed%'", params![task_run_id], |row| row.get(0)).unwrap();
        assert!(event.contains("verified_postcondition"));
        assert!(event.contains("local://teams/chat/"));
        assert!(event.contains(&result.evidence_digest));
        assert!(!event.contains("teams.chat.draft_message"));
        let _ = std::fs::remove_dir_all(root);
    }
}
