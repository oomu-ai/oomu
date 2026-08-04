use super::{
    file_io::{create_verified_text_file, verify_final_created_file},
    scheduled_path::{ensure_output_parent, resolve_registration_for_task},
    CreateFileBrief,
};
use crate::{
    projects::{CreateProjectRequest, ProjectDataPolicy},
    tools::task_tool_runtime::TASK_RUN_TIMESTAMP_TOKEN,
};
use chrono::TimeZone;
use rusqlite::params;
use serde_json::json;
use std::path::{Path, PathBuf};

fn scheduled_file_fixture(
    label: &str,
    created_at_ms: i64,
    scheduled_for_ms: i64,
    routine_timezone: &str,
) -> (
    PathBuf,
    PathBuf,
    crate::db::PersistenceEngine,
    crate::tools::task_runtime::AgentRuntimeTaskBinding,
) {
    let root = std::env::temp_dir().join(format!(
        "oomu-scheduled-create-file-{label}-{}",
        crate::p0_contracts::TaskId::new()
    ));
    let project_root = root.join("approved-project");
    std::fs::create_dir_all(&project_root).unwrap();
    let project_root = std::fs::canonicalize(project_root).unwrap();
    let engine = crate::db::PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = crate::projects::repository::create(
        &engine,
        CreateProjectRequest {
            name: format!("Scheduled file {label}"),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let task_id = crate::p0_contracts::TaskId::new().to_string();
    let task_run_id = crate::p0_contracts::TaskRunId::new().to_string();
    let workflow_id = format!("workflow-{label}");
    let execution_id = format!("execution-{label}");
    let schedule_id = format!("routine-{label}");
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES (?1,?2,'knowledge_directory',?3,?4,'active',?5,?5)",
            params![
                format!("source-{label}"),
                project.project_id,
                project_root.to_string_lossy(),
                "a".repeat(64),
                created_at_ms
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_blueprints(workflow_id,version,name,description,visual_state_json,is_active,created_at_ms,updated_at_ms) VALUES (?1,1,?2,'','{}',1,?3,?3)",
            params![workflow_id, format!("Workflow {label}"), created_at_ms],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflow_schedules(id,workflow_id,workflow_version,label,schedule_expression,is_active,created_at_ms,updated_at_ms,project_id,schedule_kind,routine_timezone) VALUES (?1,?2,1,?3,'manual',1,?4,?4,?5,'one_shot',?6)",
            params![schedule_id, workflow_id, format!("Routine {label}"), created_at_ms, project.project_id, routine_timezone],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO execution_instances(id,workflow_id,workflow_version,status,created_at_ms,updated_at_ms) VALUES (?1,?2,1,'Running',?3,?3)",
            params![execution_id, workflow_id, created_at_ms],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,?2,?3,'workflow',?4,'running','routine',?2,'Scheduled file',?5,?5,'reconciled')",
            params![
                task_run_id,
                task_id,
                project.project_id,
                execution_id,
                created_at_ms
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO routine_runs(schedule_id,execution_instance_id,task_run_id,scheduled_for_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5)",
            params![schedule_id, execution_id, task_run_id, scheduled_for_ms, created_at_ms],
        )
        .unwrap();
    drop(connection);
    let task = crate::tools::task_runtime::AgentRuntimeTaskBinding {
        task_id,
        task_run_id,
        project_id: project.project_id,
    };
    (root, project_root, engine, task)
}

#[test]
fn scheduled_resolution_uses_one_project_root_and_one_stable_task_timestamp() {
    let (root, project_root, engine, task) =
        scheduled_file_fixture("stable-path", 1_784_631_600_000, 1_784_631_600_000, "UTC");
    let request = json!({"file":{
        "title":"Supplier exception","content":"Verified evidence","locale":"en-US","format":"md",
        "destinationPath":"ship_test_06/supplier_exception_<YYYY-MM-DD_HH-mm>.md"
    }});
    let first = resolve_registration_for_task(&engine, &task, request.clone()).unwrap();
    let retry = resolve_registration_for_task(&engine, &task, request).unwrap();
    assert_eq!(first, retry);
    let path = first["file"]["destinationPath"].as_str().unwrap();
    assert!(Path::new(path).starts_with(&project_root));
    assert!(path.ends_with(".md"));
    assert!(!path.contains(TASK_RUN_TIMESTAMP_TOKEN));

    let traversal = json!({"file":{
        "title":"Escape","content":"No","locale":"en-US","format":"md",
        "destinationPath":"../outside.md"
    }});
    assert!(resolve_registration_for_task(&engine, &task, traversal)
        .unwrap_err()
        .contains("must stay inside"));

    let second = root.join("second-approved-project");
    std::fs::create_dir_all(&second).unwrap();
    let second = std::fs::canonicalize(second).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,grant_state,created_at_ms,updated_at_ms) VALUES ('source-second',?1,'local_folder',?2,?3,'active',1,1)",
            params![task.project_id, second.to_string_lossy(), "b".repeat(64)],
        )
        .unwrap();
    let explicit_project = json!({"file":{
        "title":"Project root","content":"No","locale":"en-US","format":"md",
        "destinationPath":"ship_test_06/project-root.md"
    }});
    let rebound = resolve_registration_for_task(&engine, &task, explicit_project).unwrap();
    let rebound_path = rebound["file"]["destinationPath"].as_str().unwrap();
    assert!(Path::new(rebound_path).starts_with(&second));
    assert!(rebound_path.ends_with("ship_test_06/project-root.md"));
    assert!(!Path::new(rebound_path).starts_with(&project_root));

    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scheduled_filename_uses_the_promised_occurrence_and_routine_timezone() {
    let created_at_ms = chrono::Utc
        .with_ymd_and_hms(2026, 7, 20, 15, 45, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let scheduled_for_ms = chrono::Utc
        .with_ymd_and_hms(2026, 7, 20, 15, 30, 0)
        .single()
        .unwrap()
        .timestamp_millis();
    let (root, _, engine, task) = scheduled_file_fixture(
        "promised-occurrence",
        created_at_ms,
        scheduled_for_ms,
        "America/New_York",
    );
    let resolved = resolve_registration_for_task(
        &engine,
        &task,
        json!({"file":{
            "title":"Status","content":"Verified evidence","locale":"en-US","format":"md",
            "destinationPath":"reports/status_<YYYY-MM-DD_HH-mm>.md"
        }}),
    )
    .unwrap();
    assert!(resolved["file"]["destinationPath"]
        .as_str()
        .unwrap()
        .ends_with("reports/status_2026-07-20_11-30.md"));

    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_chat_resolution_expands_home_before_project_path_binding() {
    let (root, project_root, engine, task) =
        scheduled_file_fixture("direct-home", 1_784_631_600_000, 1_784_631_600_000, "UTC");
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE task_runs SET runtime_kind='agent',origin='chat' WHERE task_run_id=?1",
            params![task.task_run_id],
        )
        .unwrap();
    let resolved = resolve_registration_for_task(
        &engine,
        &task,
        json!({"file":{
            "title":"Hello World","content":"Hello World","locale":"en-US","format":"pdf",
            "destinationPath":"~/Downloads/hello_world.pdf"
        }}),
    )
    .unwrap();
    let destination = resolved["file"]["destinationPath"].as_str().unwrap();
    let expected = std::env::var_os("HOME")
        .map(PathBuf::from)
        .expect("HOME")
        .join("Downloads/hello_world.pdf");
    assert_eq!(Path::new(destination), expected);
    assert!(!Path::new(destination).starts_with(project_root));

    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scheduled_writer_creates_one_missing_output_folder_then_verifies_real_bytes() {
    let (root, project_root, engine, task) = scheduled_file_fixture(
        "missing-folder",
        1_784_631_600_000,
        1_784_631_600_000,
        "UTC",
    );
    let destination = project_root.join("ship_test_06/supplier_exception.md");
    let mut output_parent =
        ensure_output_parent(&engine, &task.project_id, &destination.to_string_lossy()).unwrap();
    assert!(project_root.join("ship_test_06").is_dir());
    let brief = CreateFileBrief {
        title: "Supplier exception".to_string(),
        content: "Verified evidence".to_string(),
        locale: "en-US".to_string(),
        format: "md".to_string(),
        destination_path: destination.to_string_lossy().to_string(),
    };
    let created = create_verified_text_file(&brief).unwrap();
    let evidence = verify_final_created_file(&brief, &created).unwrap();
    output_parent.commit();
    assert_eq!(evidence.byte_length, "Verified evidence".len() as u64);

    let nested = project_root.join("missing-one/missing-two/report.md");
    let error =
        ensure_output_parent(&engine, &task.project_id, &nested.to_string_lossy()).unwrap_err();
    assert!(error.contains("existing parent folder"));
    assert!(!project_root.join("missing-one").exists());

    drop(engine);
    let _ = std::fs::remove_dir_all(root);
}
