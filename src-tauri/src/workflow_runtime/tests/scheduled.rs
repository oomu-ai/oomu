use super::*;

fn bounded_empty_request(end_at_ms: i64) -> Value {
    crate::routines::control::with_end_at_ms(&json!({}), end_at_ms).unwrap()
}

fn assert_internal_routine_control_was_removed(request: &RunWorkflowRequest) {
    assert!(serde_json::to_value(request)
        .unwrap()
        .get("_oomuRoutine")
        .is_none());
}

fn scheduled_project_fixture_root() -> (std::path::PathBuf, std::path::PathBuf) {
    let root =
        std::env::temp_dir().join(format!("oomu-scheduled-project-binding-{}", unix_time_ms()));
    let project_root = root.join("approved-project");
    std::fs::create_dir_all(&project_root).unwrap();
    let project_root = std::fs::canonicalize(project_root).unwrap();
    (root, project_root)
}

#[test]
fn scheduled_execution_freezes_project_root_and_binds_project_before_nodes() {
    let (root, project_root) = scheduled_project_fixture_root();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let (project_id, now) = (
        "project_00000000-0000-4000-8000-000000000321",
        unix_time_ms(),
    );
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO projects(project_id,name,description,created_at_ms,updated_at_ms) VALUES (?1,'Scheduled Project','',?2,?2)",
            rusqlite::params![project_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_policy(project_id,data_policy,updated_at_ms) VALUES (?1,'local_only',?2)",
            rusqlite::params![project_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,created_at_ms,updated_at_ms) VALUES ('source-scheduled',?1,'knowledge_directory',?2,?3,?4,?4)",
            rusqlite::params![project_id, project_root.to_string_lossy(), "a".repeat(64), now],
        )
        .unwrap();
    drop(connection);

    let mut compiled = compiled_workflow(false);
    let saved = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: "{}".to_string(),
        created_at: now,
        updated_at: now,
    };
    engine
        .reserve_workflow_blueprint(&saved, &json!({}), &mut compiled.workflow_ir)
        .unwrap();
    engine
        .publish_compiled_workflow(
            &saved,
            &compiled.workflow_ir,
            &compiled.instructions.into_values().collect::<Vec<_>>(),
            true,
        )
        .unwrap();
    let scheduled_for_ms = now + 60_000;
    let schedule = engine
        .upsert_workflow_schedule(crate::db::WorkflowScheduleUpsert {
            id: "routine_project_binding".to_string(),
            workflow_id: saved.id,
            workflow_version: Some(compiled.workflow_ir.workflow_version),
            label: "Project binding".to_string(),
            schedule_expression: format!("once:{scheduled_for_ms}"),
            run_request: bounded_empty_request(scheduled_for_ms + 60_000),
            is_active: true,
            next_run_at_ms: Some(scheduled_for_ms),
        })
        .unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "UPDATE workflow_schedules SET project_id=?2,schedule_kind='one_shot',routine_timezone='UTC' WHERE id=?1",
            rusqlite::params![schedule.id, project_id],
        )
        .unwrap();
    let schedule = engine.load_workflow_schedule(&schedule.id).unwrap();
    let context = resolve_scheduled_project_context(&schedule, &engine).unwrap();
    assert_eq!(context.project_id, project_id);
    assert_eq!(context.project_root, project_root);
    assert_eq!(context.scheduled_for_ms, Some(scheduled_for_ms));

    let compiled = engine
        .load_compiled_workflow(&schedule.workflow_id, schedule.workflow_version)
        .unwrap();
    let request = scheduled_run_request(&schedule, &compiled, &context).unwrap();
    assert_internal_routine_control_was_removed(&request);
    let InputBinding::Manual { value } = request.inputs.get("input").unwrap() else {
        panic!("scheduled input must be a frozen manual binding")
    };
    assert_eq!(value["projectId"], project_id);
    assert_eq!(
        value["projectRoot"],
        project_root.to_string_lossy().as_ref()
    );
    assert_eq!(value["scheduledAtMs"], scheduled_for_ms);

    let instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    engine
        .insert_scheduled_execution_instance(
            &instance,
            project_id,
            &schedule.id,
            Some(scheduled_for_ms),
        )
        .unwrap();
    let connection = engine.open_connection().unwrap();
    let bound_project: String = connection
        .query_row(
            "SELECT project_id FROM execution_instances WHERE id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    let frozen_occurrence: i64 = connection
        .query_row(
            "SELECT scheduled_for_ms FROM routine_runs WHERE execution_instance_id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    let (task_project, task_state): (String, String) = connection
        .query_row(
            "SELECT project_id,state FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let linked_task_run_id: String = connection
        .query_row(
            "SELECT task_run_id FROM routine_runs WHERE execution_instance_id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bound_project, project_id);
    assert_eq!(frozen_occurrence, scheduled_for_ms);
    assert_eq!(task_project, project_id);
    assert_eq!(task_state, "running");
    assert!(linked_task_run_id.starts_with("taskrun_"));
    drop(connection);

    let second_root = root.join("second-approved-project");
    std::fs::create_dir_all(&second_root).unwrap();
    let second_root = std::fs::canonicalize(second_root).unwrap();
    engine
        .open_connection()
        .unwrap()
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,created_at_ms,updated_at_ms) VALUES ('source-scheduled-second',?1,'local_folder',?2,?3,?4,?4)",
            rusqlite::params![project_id, second_root.to_string_lossy(), "b".repeat(64), now],
        )
        .unwrap();
    let rebound = resolve_scheduled_project_context(&schedule, &engine).unwrap();
    assert_eq!(rebound.project_root, second_root);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_execution_binds_compiled_project_and_task_before_project_file_read() {
    let root = std::env::temp_dir().join(format!("oomu-direct-project-binding-{}", unix_time_ms()));
    let project_root = root.join("approved-project");
    std::fs::create_dir_all(&project_root).unwrap();
    let fixture = project_root.join("supplier_proposals.json");
    std::fs::write(&fixture, br#"{"supplier":"Apex Cargo"}"#).unwrap();
    let project_root = std::fs::canonicalize(project_root).unwrap();
    let fixture = std::fs::canonicalize(fixture).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project_id = "project_00000000-0000-4000-8000-000000000322";
    let now = unix_time_ms();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "INSERT INTO projects(project_id,name,description,created_at_ms,updated_at_ms) VALUES (?1,'Direct Project','',?2,?2)",
            rusqlite::params![project_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_policy(project_id,data_policy,updated_at_ms) VALUES (?1,'local_only',?2)",
            rusqlite::params![project_id, now],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO project_sources(source_id,project_id,source_kind,canonical_path,grant_reference,created_at_ms,updated_at_ms) VALUES ('source-direct',?1,'knowledge_directory',?2,?3,?4,?4)",
            rusqlite::params![project_id, project_root.to_string_lossy(), "c".repeat(64), now],
        )
        .unwrap();
    drop(connection);

    let mut compiled = compiled_workflow(false);
    let saved = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: "{}".to_string(),
        created_at: now,
        updated_at: now,
    };
    let (version, binding) = engine
        .reserve_workflow_blueprint_for_project(
            &saved,
            &json!({}),
            &mut compiled.workflow_ir,
            Some(project_id),
        )
        .unwrap();
    assert_eq!(binding.as_deref(), Some(project_id));
    engine
        .publish_compiled_workflow_for_project(
            &saved,
            &json!({}),
            &compiled.workflow_ir,
            &compiled.instructions.into_values().collect::<Vec<_>>(),
            true,
            Some(project_id),
        )
        .unwrap();
    let request = RunWorkflowRequest {
        workflow_id: saved.id,
        workflow_version: Some(version),
        preflight_mode: WorkflowPreflightMode::Skipped,
        inputs: HashMap::new(),
        outputs: HashMap::new(),
    };
    let instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    assert_eq!(
        engine
            .insert_direct_workflow_execution_instance(&instance)
            .unwrap()
            .as_deref(),
        Some(project_id)
    );

    let (task_run_id, task_project, runtime_kind, origin): (String, String, String, String) = engine
        .open_connection()
        .unwrap()
        .query_row(
            "SELECT task_run_id,project_id,runtime_kind,origin FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    crate::tasks::require_bound_task(&engine, &task_run_id, project_id).unwrap();
    assert_eq!(task_project, project_id);
    assert_eq!(runtime_kind, "workflow");
    assert_eq!(origin, "workflow");
    let receipt = crate::tools::project_file::read_project_file(
        &engine,
        project_id,
        fixture.to_string_lossy().as_ref(),
        1024,
    )
    .unwrap();
    assert_eq!(receipt.canonical_path, fixture.to_string_lossy());
    assert!(receipt.verified);
    assert!(receipt.byte_count > 0);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_global_workflow_remains_unbound() {
    let root = std::env::temp_dir().join(format!("oomu-direct-global-binding-{}", unix_time_ms()));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let now = unix_time_ms();
    let mut compiled = compiled_workflow(false);
    let saved = SavedWorkflowRecord {
        id: compiled.workflow_ir.workflow_id.clone(),
        name: compiled.workflow_ir.name.clone(),
        steps: "{}".to_string(),
        created_at: now,
        updated_at: now,
    };
    let version = engine
        .reserve_workflow_blueprint(&saved, &json!({}), &mut compiled.workflow_ir)
        .unwrap();
    engine
        .publish_compiled_workflow(
            &saved,
            &compiled.workflow_ir,
            &compiled.instructions.into_values().collect::<Vec<_>>(),
            true,
        )
        .unwrap();
    let request = RunWorkflowRequest {
        workflow_id: saved.id,
        workflow_version: Some(version),
        preflight_mode: WorkflowPreflightMode::Skipped,
        inputs: HashMap::new(),
        outputs: HashMap::new(),
    };
    let instance = new_instance(&compiled.workflow_ir, &request).unwrap();
    assert_eq!(
        engine
            .insert_direct_workflow_execution_instance(&instance)
            .unwrap(),
        None
    );
    let connection = engine.open_connection().unwrap();
    let project: Option<String> = connection
        .query_row(
            "SELECT project_id FROM execution_instances WHERE id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    let task_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM task_runs WHERE runtime_kind='workflow' AND runtime_record_id=?1",
            rusqlite::params![instance.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(project, None);
    assert_eq!(task_count, 0);

    std::fs::remove_dir_all(root).unwrap();
}
