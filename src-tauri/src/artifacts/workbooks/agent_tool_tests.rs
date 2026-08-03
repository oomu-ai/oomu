use super::*;
use crate::{
    db::CreateChatSessionRequest,
    p0_contracts::{EvidenceClass, TaskRunId},
    projects::{BindProjectRecordRequest, CreateProjectRequest, ProjectDataPolicy},
};
use rusqlite::params;

fn projection() -> SpreadsheetSourceProjection {
    SpreadsheetSourceProjection {
        from_step: 0,
        collection_pointer: "/result/value".to_string(),
        title: "Observed messages".to_string(),
        locale: "en-US".to_string(),
        sheet_name: "Messages".to_string(),
        columns: vec![
            SpreadsheetProjectionColumn {
                header: "Subject".to_string(),
                field: "subject".to_string(),
            },
            SpreadsheetProjectionColumn {
                header: "Unread".to_string(),
                field: "unread".to_string(),
            },
        ],
    }
}

fn source_request() -> Value {
    serde_json::to_value(CreateSpreadsheetToolRequest::Source(
        SourceProjectionRequest {
            source_projection: projection(),
        },
    ))
    .unwrap()
}

fn connected_output(result: &Value, evidence_ref: &str, digest: &str) -> ExecuteCommandResponse {
    ExecuteCommandResponse {
        operation: "connected_work".to_string(),
        status: CommandStatus::Completed,
        message: json!({
            "result":result,
            "partial":false,
            "sourceRef":"connector.tool.completed",
            "evidenceRef":evidence_ref,
            "evidenceDigest":digest,
        })
        .to_string(),
        metrics: None,
        claims: vec!["CLAIM connector_task_evidence evidence_recorded=true".to_string()],
        verified: true,
        model_used: None,
    }
}

#[test]
fn observed_rows_project_exact_scalars_with_canonical_lineage() {
    let source = json!({"result":{"value":[
        {"subject":"Quarterly review","unread":true},
        {"subject":"Budget follow-up","unread":false}
    ]}});
    let workbook = project_rows(
        &projection(),
        &source,
        "connector.tool.completed",
        "task-event:taskrun_00000000-0000-4000-8000-000000000000:7",
    )
    .unwrap();
    assert_eq!(
        workbook.worksheets[0].cells[2].value,
        CellValue::Text {
            value: "Quarterly review".to_string()
        }
    );
    assert_eq!(
        workbook.worksheets[0].cells[4].value,
        CellValue::Text {
            value: "Budget follow-up".to_string()
        }
    );
    for cell in workbook.worksheets[0].cells.iter().skip(2) {
        assert_eq!(cell.provenance.len(), 1);
        assert_eq!(cell.provenance[0].source_ref, "connector.tool.completed");
    }
}

#[test]
fn planner_json_parses_and_validates_without_rust_round_trip() {
    let request = json!({
        "workbook": {
            "schemaVersion": 1,
            "title": "Planner workbook",
            "locale": "en-US",
            "dateSystem": "1900",
            "revision": 1,
            "worksheets": [{
                "sheetId": "planner_sheet",
                "name": "Summary",
                "bounds": {"rowCount": 10, "columnCount": 4},
                "cells": [
                    {"address":"A1","value":{"kind":"text","value":"Category"}},
                    {"address":"B1","value":{"kind":"text","value":"Value"}},
                    {"address":"A2","value":{"kind":"text","value":"Q1"}},
                    {"address":"B2","value":{"kind":"number","value":10}},
                    {"address":"B3","value":{"kind":"formula","expression":"SUM(B2:B2)"}}
                ],
                "validations": [{
                    "validationId":"category_list",
                    "range":"A2:A2",
                    "rule":{"kind":"list","values":["Q1","Q2"]}
                }],
                "charts": [{
                    "chartId":"value_chart",
                    "kind":"column",
                    "title":"Values",
                    "categoryRange":"Summary!A2:A2",
                    "series":[{"name":"Value","valueRange":"Summary!B2:B2"}],
                    "anchor":{"fromColumn":1,"fromRow":4,"toColumn":4,"toRow":10}
                }]
            }],
            "recalculation":{"status":"stale"}
        }
    });
    let validated = validate_registration(request).unwrap();
    let parsed =
        serde_json::from_value::<CreateSpreadsheetToolRequest>(validated.arguments).unwrap();
    assert!(matches!(parsed, CreateSpreadsheetToolRequest::Direct(_)));
}

#[test]
fn direct_workbook_cannot_fabricate_observed_provenance() {
    let source = json!({"result":{"value":[{"subject":"Observed","unread":true}]}});
    let workbook = project_rows(
        &projection(),
        &source,
        "connector.tool.completed",
        "task-event:taskrun_00000000-0000-4000-8000-000000000000:7",
    )
    .unwrap();
    assert!(
        validate_public_request(CreateSpreadsheetToolRequest::Direct(
            DirectWorkbookRequest { workbook }
        ))
        .is_err()
    );
}

#[test]
fn registry_schema_advertises_and_runtime_validates_both_exclusive_forms() {
    let _ = register_task_tool();
    let payload = crate::tools::registry::native_tool_schema_payload(
        crate::tools::registry::ModelProvider::LocalGemmaIt,
    );
    let schema = payload["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["kind"] == "create_spreadsheet")
        .unwrap()
        .get("inputSchema")
        .unwrap();
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
    assert_eq!(
        schema["properties"]["sourceProjection"]["required"],
        json!([
            "fromStep",
            "collectionPointer",
            "title",
            "locale",
            "sheetName",
            "columns"
        ])
    );

    let direct = json!({"workbook": empty_workbook_placeholder()});
    let source = source_request();
    assert!(validate_registration(direct.clone()).is_ok());
    assert!(validate_registration(source.clone()).is_ok());
    let mut ambiguous = direct.as_object().unwrap().clone();
    ambiguous.insert(
        "sourceProjection".to_string(),
        source["sourceProjection"].clone(),
    );
    assert!(validate_registration(Value::Object(ambiguous)).is_err());
    let mut extra = source;
    extra["sourceProjection"]["unexpected"] = json!(true);
    assert!(validate_registration(extra).is_err());
    assert!(validate_registration(json!({})).is_err());
}

#[test]
fn source_projection_resolves_only_canonical_task_bound_connector_evidence() {
    crate::tasks::register_runtime_bridge().unwrap();
    let root = std::env::temp_dir().join(format!(
        "oomu-workbook-source-projection-{}",
        crate::foundation::clock::unix_time_ns_u128()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    engine.open_connection().unwrap().execute_batch(
        "CREATE TABLE taskflows (flow_id TEXT PRIMARY KEY,parent_session_id TEXT NOT NULL,directive TEXT NOT NULL,status TEXT NOT NULL,created_at_ms INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL); CREATE TABLE taskflow_steps (flow_id TEXT NOT NULL,status TEXT NOT NULL);",
    ).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Source projection".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let other_project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: "Wrong project".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let session = engine
        .ensure_chat_session(CreateChatSessionRequest {
            agent_id: "agent-source-projection".to_string(),
            provider_id: "local_model".to_string(),
            model_id: "model-test".to_string(),
            title: Some("Observed messages".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    crate::projects::repository::bind_record(
        &engine,
        BindProjectRecordRequest {
            project_id: Some(project.project_id.clone()),
            record_kind: "chat_session".to_string(),
            record_id: session.id.clone(),
        },
    )
    .unwrap();
    let execution_id = "execution-source-projection";
    let now = crate::foundation::clock::unix_time_ms_i64();
    engine.open_connection().unwrap().execute(
        "INSERT INTO agent_executions (execution_id,plan_id,session_id,agent_id,provider_id,model_id,turn_id,generation_token,root_turn_id,turn_kind,context_json,status,created_at_ms,updated_at_ms) VALUES (?1,'plan-source',?2,'agent-source-projection','local_model','model-test','turn-source','generation-source','turn-source','user','{}','running',?3,?3)",
        params![execution_id,session.id,now],
    ).unwrap();
    let task =
        crate::tools::task_runtime::require_agent_runtime_task(&engine, execution_id).unwrap();
    assert_eq!(task.project_id, project.project_id);

    let result = json!({"value":[
        {"subject":"Quarterly review","unread":true},
        {"subject":"Budget follow-up","unread":false}
    ]});
    let digest = sha256_hex(&serde_json::to_vec(&result).unwrap());
    let sequence = crate::tools::task_runtime::record_event_with_sequence(
        &engine,
        &task.task_run_id,
        "connector.tool.completed",
        EvidenceClass::ObservedResult,
        json!({
            "resultDigest":digest,
            "resultExcerpt":result,
            "partial":false,
        }),
    )
    .unwrap();
    let evidence_ref = format!("task-event:{}:{sequence}", task.task_run_id);
    let output = connected_output(&result, &evidence_ref, &digest);
    let resolved_value =
        resolve_registration(&engine, Some(execution_id), source_request(), &[output]).unwrap();
    let resolved =
        serde_json::from_value::<CreateSpreadsheetToolRequest>(resolved_value.clone()).unwrap();
    let workbook = match resolved {
        CreateSpreadsheetToolRequest::Resolved(request) => {
            request.resolved_source_projection.workbook
        }
        _ => panic!("source projection did not resolve"),
    };
    assert_eq!(
        workbook.worksheets[0].cells[2].value,
        CellValue::Text {
            value: "Quarterly review".to_string()
        }
    );
    assert_eq!(
        workbook.worksheets[0].cells[3].value,
        CellValue::Boolean { value: true }
    );
    for cell in workbook.worksheets[0].cells.iter().skip(2) {
        assert_eq!(cell.provenance.len(), 1);
        assert_eq!(cell.provenance[0].source_ref, "connector.tool.completed");
        assert_eq!(cell.provenance[0].evidence_ref, evidence_ref);
    }

    let wrong_task_ref = format!("task-event:{}:{sequence}", TaskRunId::new());
    let error = resolve_registration(
        &engine,
        Some(execution_id),
        source_request(),
        &[connected_output(&result, &wrong_task_ref, &digest)],
    )
    .unwrap_err();
    assert_eq!(error, "workbook_source_evidence_ref_invalid");

    let event_json: String = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT event_json FROM task_events WHERE task_run_id=?1 AND sequence=?2",
            params![task.task_run_id, sequence as i64],
            |row| row.get(0),
        )
        .unwrap();
    let bad_sequence = sequence + 1;
    let mut wrong_project_event: Value = serde_json::from_str(&event_json).unwrap();
    wrong_project_event["sequence"] = json!(bad_sequence);
    wrong_project_event["projectId"] = json!(other_project.project_id);
    engine.open_connection().unwrap().execute(
        "INSERT INTO task_events (task_run_id,sequence,event_json,created_at_ms) VALUES (?1,?2,?3,?4)",
        params![task.task_run_id,bad_sequence as i64,wrong_project_event.to_string(),now],
    ).unwrap();
    let wrong_project_ref = format!("task-event:{}:{bad_sequence}", task.task_run_id);
    let error = resolve_registration(
        &engine,
        Some(execution_id),
        source_request(),
        &[connected_output(&result, &wrong_project_ref, &digest)],
    )
    .unwrap_err();
    assert_eq!(error, "workbook_source_evidence_binding_mismatch");

    let wrong_digest = "0".repeat(64);
    let error = resolve_registration(
        &engine,
        Some(execution_id),
        source_request(),
        &[connected_output(&result, &evidence_ref, &wrong_digest)],
    )
    .unwrap_err();
    assert_eq!(error, "workbook_source_evidence_binding_mismatch");
    assert!(validate_registration(resolved_value).is_err());
    let _ = std::fs::remove_dir_all(root);
}
