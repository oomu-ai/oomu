use super::{
    repository::{
        archive, attach_source, bind_record, delete, deletion_preview,
        ensure_internal_local_files_project, list_sources, refresh_source, revoke_source,
        set_instructions, set_policy, update, INTERNAL_LOCAL_FILES_PROJECT_ID,
    },
    AttachProjectSourceRequest, BindProjectRecordRequest, DeleteProjectRequest, ProjectDataPolicy,
    ProjectSourceRequest, SetProjectInstructionsRequest, SetProjectPolicyRequest,
    UpdateProjectRequest,
};
use crate::db::PersistenceEngine;
use rusqlite::params;

const MANAGED_ERROR: &str = "This private OOMU workspace is managed automatically.";

fn assert_managed<T>(result: Result<T, String>) {
    assert_eq!(result.err().as_deref(), Some(MANAGED_ERROR));
}

fn fixture(label: &str) -> (std::path::PathBuf, PersistenceEngine) {
    let root = std::env::temp_dir().join(format!(
        "oomu-reserved-project-{label}-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    ensure_internal_local_files_project(&connection).unwrap();
    drop(connection);
    (root, engine)
}

#[test]
fn ensure_private_local_files_project_repairs_its_canonical_state() {
    let (root, engine) = fixture("repair");
    let connection = engine.open_connection().unwrap();
    connection
        .execute(
            "UPDATE projects SET name='Changed',description='Changed',archived_at_ms=1 WHERE project_id=?1",
            params![INTERNAL_LOCAL_FILES_PROJECT_ID],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE project_policy SET data_policy='allow_configured_cloud' WHERE project_id=?1",
            params![INTERNAL_LOCAL_FILES_PROJECT_ID],
        )
        .unwrap();

    ensure_internal_local_files_project(&connection).unwrap();

    let state: (String, String, Option<i64>, String) = connection
        .query_row(
            "SELECT p.name,p.description,p.archived_at_ms,policy.data_policy
             FROM projects p JOIN project_policy policy ON policy.project_id=p.project_id
             WHERE p.project_id=?1",
            params![INTERNAL_LOCAL_FILES_PROJECT_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "My files".to_string(),
            "Private workspace used for files created from Chat.".to_string(),
            None,
            "local_only".to_string()
        )
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn private_local_files_project_rejects_user_project_changes_sources_and_bindings() {
    let (root, engine) = fixture("boundaries");
    let app_data = root.join("app-data");
    std::fs::create_dir_all(&app_data).unwrap();

    assert_managed(update(
        &engine,
        UpdateProjectRequest {
            project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
            name: "Changed".to_string(),
            description: "Changed".to_string(),
        },
    ));
    assert_managed(archive(&engine, INTERNAL_LOCAL_FILES_PROJECT_ID));
    assert_managed(deletion_preview(
        &engine,
        INTERNAL_LOCAL_FILES_PROJECT_ID,
        &app_data,
    ));
    assert_managed(attach_source(
        &engine,
        AttachProjectSourceRequest {
            project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
            path: root.to_string_lossy().to_string(),
            grant_reference: "a".repeat(64),
            source_kind: "local_folder".to_string(),
        },
    ));
    assert_managed(list_sources(&engine, INTERNAL_LOCAL_FILES_PROJECT_ID));
    let source_request = ProjectSourceRequest {
        project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
        source_id: "source_00000000-0000-4000-8000-000000000001".to_string(),
    };
    assert_managed(refresh_source(&engine, source_request.clone()));
    assert_managed(revoke_source(&engine, source_request));
    assert_managed(set_instructions(
        &engine,
        SetProjectInstructionsRequest {
            project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
            instructions: "Changed".to_string(),
        },
    ));
    assert_managed(set_policy(
        &engine,
        SetProjectPolicyRequest {
            project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
            data_policy: ProjectDataPolicy::AllowConfiguredCloud,
        },
    ));
    assert_managed(bind_record(
        &engine,
        BindProjectRecordRequest {
            project_id: Some(INTERNAL_LOCAL_FILES_PROJECT_ID.to_string()),
            record_kind: "chat_session".to_string(),
            record_id: "session-does-not-matter".to_string(),
        },
    ));

    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let memory = crate::memory_ledger::MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
    assert_managed(delete(
        &engine,
        &knowledge,
        &memory,
        &app_data,
        DeleteProjectRequest {
            project_id: INTERNAL_LOCAL_FILES_PROJECT_ID.to_string(),
            permanently_remove_project_record: true,
            detach_dependents: true,
            delete_project_files: true,
        },
    ));

    let connection = engine.open_connection().unwrap();
    let state: (String, String, Option<i64>, String) = connection
        .query_row(
            "SELECT p.name,p.description,p.archived_at_ms,policy.data_policy
             FROM projects p JOIN project_policy policy ON policy.project_id=p.project_id
             WHERE p.project_id=?1",
            params![INTERNAL_LOCAL_FILES_PROJECT_ID],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "My files".to_string(),
            "Private workspace used for files created from Chat.".to_string(),
            None,
            "local_only".to_string()
        )
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
