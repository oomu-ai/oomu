use super::repository::{attach_source, create, delete, deletion_preview, get};
use super::*;
use crate::{db::PersistenceEngine, p0_contracts::ProjectId};
use rusqlite::params;
use std::fs;

#[test]
fn permanent_delete_removes_private_project_data_and_detaches_linked_work() {
    let root = std::env::temp_dir().join(format!(
        "oomu-project-permanent-delete-{}-{}",
        crate::foundation::clock::unix_time_ms_i64(),
        ProjectId::new()
    ));
    let app_data = root.join("app-data");
    let external = root.join("external-source");
    fs::create_dir_all(&app_data).unwrap();
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join("keep.md"), "user-owned source").unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let memory = crate::memory_ledger::MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
    let project = create(
        &engine,
        CreateProjectRequest {
            name: "Delete me".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    attach_source(
        &engine,
        AttachProjectSourceRequest {
            project_id: project.project_id.clone(),
            path: external.to_string_lossy().to_string(),
            grant_reference: "b".repeat(64),
            source_kind: "knowledge_directory".to_string(),
        },
    )
    .unwrap();

    let session = engine
        .ensure_chat_session(crate::db::CreateChatSessionRequest {
            agent_id: "agent-test".to_string(),
            provider_id: "local".to_string(),
            model_id: "test".to_string(),
            title: Some("Project chat".to_string()),
            dynamic_routing_override: None,
            workspace_id: None,
        })
        .unwrap();
    let task_run_id = "taskrun_11111111-1111-4111-8111-111111111111";
    let artifact_id = "artifact_11111111-1111-4111-8111-111111111111";
    let artifact_root = app_data
        .join("artifacts")
        .join("staging")
        .join(artifact_id)
        .join("v1");
    fs::create_dir_all(&artifact_root).unwrap();
    fs::write(artifact_root.join("brief.docx"), "private docx").unwrap();
    fs::write(artifact_root.join("brief.pdf"), "private pdf").unwrap();
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE chat_sessions SET project_id=?2 WHERE id=?1",
            params![session.id, project.project_id],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO workflows(id,name,steps,created_at,updated_at,project_id) VALUES ('workflow-delete','Workflow','[]',1,1,?1)",
            params![project.project_id],
        )
        .unwrap();
    connection.execute(
        "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,'task_11111111-1111-4111-8111-111111111111',?2,'agent','runtime-delete','completed','chat','correlation-delete','Saved task',1,1,'not_required')",
        params![task_run_id, project.project_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO artifact_records(artifact_id,project_id,task_run_id,title,current_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,'Private brief',1,1,1)",
        params![artifact_id,project.project_id,task_run_id],
    ).unwrap();
    drop(connection);

    let preview = deletion_preview(&engine, &project.project_id, &app_data).unwrap();
    assert_eq!(preview.user_files_to_delete, 2);
    assert_eq!(preview.conversations_to_detach, 1);
    assert_eq!(preview.task_runs_to_detach, 1);

    delete(
        &engine,
        &knowledge,
        &memory,
        &app_data,
        DeleteProjectRequest {
            project_id: project.project_id.clone(),
            permanently_remove_project_record: true,
            detach_dependents: true,
            delete_project_files: true,
        },
    )
    .unwrap();

    assert!(get(&engine, &project.project_id).is_err());
    assert!(!app_data
        .join("artifacts")
        .join("staging")
        .join(artifact_id)
        .exists());
    assert_eq!(
        fs::read_to_string(external.join("keep.md")).unwrap(),
        "user-owned source"
    );
    let connection = engine.open_connection().unwrap();
    for table in [
        "projects",
        "project_sources",
        "project_instructions",
        "project_policy",
        "artifact_records",
    ] {
        let count: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE project_id=?1"),
                params![project.project_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table} retained project-owned data");
    }
    let chat_project: Option<String> = connection
        .query_row(
            "SELECT project_id FROM chat_sessions WHERE id=?1",
            params![session.id],
            |row| row.get(0),
        )
        .unwrap();
    let workflow_project: Option<String> = connection
        .query_row(
            "SELECT project_id FROM workflows WHERE id='workflow-delete'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let task_project: Option<String> = connection
        .query_row(
            "SELECT project_id FROM task_runs WHERE task_run_id=?1",
            params![task_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chat_project, None);
    assert_eq!(workflow_project, None);
    assert_eq!(task_project, None);
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn permanent_delete_requires_explicit_project_file_confirmation() {
    let root = std::env::temp_dir().join(format!(
        "oomu-project-delete-confirmation-{}",
        ProjectId::new()
    ));
    let app_data = root.join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let memory = crate::memory_ledger::MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
    let project = create(
        &engine,
        CreateProjectRequest {
            name: "Keep without confirmation".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let error = delete(
        &engine,
        &knowledge,
        &memory,
        &app_data,
        DeleteProjectRequest {
            project_id: project.project_id.clone(),
            permanently_remove_project_record: true,
            detach_dependents: true,
            delete_project_files: false,
        },
    )
    .unwrap_err();
    assert!(error.contains("project-file deletion"));
    assert!(get(&engine, &project.project_id).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unsafe_recorded_private_path_fails_closed() {
    let root = std::env::temp_dir().join(format!(
        "oomu-project-delete-unsafe-path-{}",
        ProjectId::new()
    ));
    let app_data = root.join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let external_file = root.join("must-survive.docx");
    fs::write(&external_file, "outside OOMU private storage").unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let memory = crate::memory_ledger::MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
    let project = create(
        &engine,
        CreateProjectRequest {
            name: "Unsafe path".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let artifact_id = "artifact_22222222-2222-4222-8222-222222222222";
    let task_run_id = "taskrun_22222222-2222-4222-8222-222222222222";
    let connection = engine.open_connection().unwrap();
    connection.execute(
        "INSERT INTO task_runs(task_run_id,task_id,project_id,runtime_kind,runtime_record_id,state,origin,correlation_id,summary,created_at_ms,updated_at_ms,recovery_state) VALUES (?1,'task_22222222-2222-4222-8222-222222222222',?2,'agent','runtime-unsafe','completed','chat','correlation-unsafe','Unsafe task',1,1,'not_required')",
        params![task_run_id, project.project_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO artifact_records(artifact_id,project_id,task_run_id,title,current_version,created_at_ms,updated_at_ms) VALUES (?1,?2,?3,'Unsafe',1,1,1)",
        params![artifact_id,project.project_id,task_run_id],
    ).unwrap();
    connection.execute(
        "INSERT INTO artifact_versions(artifact_id,version,document_json,status,docx_private_path,preview_manifest_json,verification_json,provenance_json,builder_identity,created_at_ms) VALUES (?1,1,'{}','verified',?2,'[]','{}','{}','test',1)",
        params![artifact_id,external_file.to_string_lossy()],
    ).unwrap();
    drop(connection);

    let error = delete(
        &engine,
        &knowledge,
        &memory,
        &app_data,
        DeleteProjectRequest {
            project_id: project.project_id.clone(),
            permanently_remove_project_record: true,
            detach_dependents: true,
            delete_project_files: true,
        },
    )
    .unwrap_err();
    assert!(error.contains("outside its OOMU-owned directory"));
    assert!(get(&engine, &project.project_id).is_ok());
    assert_eq!(
        fs::read_to_string(&external_file).unwrap(),
        "outside OOMU private storage"
    );
    let _ = fs::remove_dir_all(root);
}
