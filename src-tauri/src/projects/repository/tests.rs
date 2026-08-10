use super::*;
use std::fs;

#[test]
fn project_names_and_grants_fail_closed() {
    assert!(clean_text("", "name", 120, true).is_err());
    assert!(clean_text(&"x".repeat(121), "name", 120, true).is_err());
}

#[test]
fn private_local_files_project_is_real_local_only_and_hidden_from_project_lists() {
    let root = std::env::temp_dir().join(format!(
        "oomu-private-local-files-project-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let connection = engine.open_connection().unwrap();
    let project_id = ensure_internal_local_files_project(&connection).unwrap();
    assert_eq!(project_id, INTERNAL_LOCAL_FILES_PROJECT_ID);
    let policy: String = connection
        .query_row(
            "SELECT data_policy FROM project_policy WHERE project_id=?1",
            params![project_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(policy, "local_only");
    drop(connection);
    assert!(list(&engine, false).unwrap().is_empty());
    assert!(list(&engine, true).unwrap().is_empty());
    assert_eq!(
        get(&engine, INTERNAL_LOCAL_FILES_PROJECT_ID).unwrap().name,
        "My files"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn projects_isolate_sources_and_archive_without_deleting_files() {
    let root = std::env::temp_dir().join(format!(
        "oomu-project-domain-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    fs::create_dir_all(root.join("source-a")).unwrap();
    fs::write(root.join("source-a").join("notes.md"), "project knowledge").unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let memory = crate::memory_ledger::MemoryLedger::initialize_at(root.join("memory.db")).unwrap();
    let app_data = root.join("app-data");
    fs::create_dir_all(&app_data).unwrap();
    let first = create(
        &engine,
        CreateProjectRequest {
            name: "First".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let second = create(
        &engine,
        CreateProjectRequest {
            name: "Second".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::AllowConfiguredCloud,
        },
    )
    .unwrap();
    let source = attach_source(
        &engine,
        AttachProjectSourceRequest {
            project_id: first.project_id.clone(),
            path: root.join("source-a").to_string_lossy().to_string(),
            grant_reference: "a".repeat(64),
            source_kind: "knowledge_directory".to_string(),
        },
    )
    .unwrap();
    assert_eq!(list_sources(&engine, &first.project_id).unwrap().len(), 1);
    assert!(list_sources(&engine, &second.project_id)
        .unwrap()
        .is_empty());
    assert_eq!(
        refresh_source(
            &engine,
            ProjectSourceRequest {
                project_id: first.project_id.clone(),
                source_id: source.source_id,
            },
        )
        .unwrap()
        .file_count,
        1
    );
    delete(
        &engine,
        &knowledge,
        &memory,
        &app_data,
        DeleteProjectRequest {
            project_id: first.project_id.clone(),
            permanently_remove_project_record: false,
            detach_dependents: false,
            delete_project_files: false,
        },
    )
    .unwrap();
    assert!(root.join("source-a").join("notes.md").exists());
    assert!(get(&engine, &first.project_id)
        .unwrap()
        .archived_at_ms
        .is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn picked_project_root_is_single_replaceable_and_distinct_from_knowledge() {
    let root = std::env::temp_dir().join(format!(
        "oomu-project-root-picker-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    for name in ["knowledge-a", "knowledge-b", "root-a", "root-b"] {
        fs::create_dir_all(root.join(name)).unwrap();
    }
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let project = create(
        &engine,
        CreateProjectRequest {
            name: "Root test".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    for name in ["knowledge-a", "knowledge-b"] {
        attach_source(
            &engine,
            AttachProjectSourceRequest {
                project_id: project.project_id.clone(),
                path: root.join(name).to_string_lossy().to_string(),
                grant_reference: "a".repeat(64),
                source_kind: "knowledge_directory".to_string(),
            },
        )
        .unwrap();
    }
    let first = attach_picked_root(&engine, &project.project_id, &root.join("root-a")).unwrap();
    assert_eq!(first.source_kind, "local_folder");
    assert_eq!(first.indexing_state, "ready");
    assert_eq!(first.file_count, 0);

    let changed = attach_picked_root(&engine, &project.project_id, &root.join("root-b")).unwrap();
    assert_eq!(changed.source_id, first.source_id);
    let sources = list_sources(&engine, &project.project_id).unwrap();
    assert_eq!(
        sources
            .iter()
            .filter(|source| source.source_kind == "local_folder")
            .count(),
        1
    );
    assert_eq!(
        crate::projects::path_scope::single_active_project_root(&engine, &project.project_id)
            .unwrap(),
        fs::canonicalize(root.join("root-b")).unwrap()
    );
    let evidence_roots =
        crate::projects::path_scope::active_project_evidence_roots(&engine, &project.project_id)
            .unwrap();
    assert_eq!(
        evidence_roots,
        vec![fs::canonicalize(root.join("root-b")).unwrap()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn startup_refresh_keeps_empty_knowledge_folders_ready() {
    let root = std::env::temp_dir().join(format!(
        "oomu-empty-project-knowledge-{}",
        crate::foundation::clock::unix_time_ms_i64()
    ));
    fs::create_dir_all(root.join("empty-source")).unwrap();
    let engine = PersistenceEngine::initialize_at(root.join("state.sqlite")).unwrap();
    let knowledge =
        crate::knowledge::KnowledgeStore::initialize_at(root.join("knowledge.db")).unwrap();
    let project = create(
        &engine,
        CreateProjectRequest {
            name: "Empty knowledge".to_string(),
            description: String::new(),
            data_policy: ProjectDataPolicy::LocalOnly,
        },
    )
    .unwrap();
    let source = attach_source(
        &engine,
        AttachProjectSourceRequest {
            project_id: project.project_id.clone(),
            path: root.join("empty-source").to_string_lossy().to_string(),
            grant_reference: "b".repeat(64),
            source_kind: "knowledge_directory".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        crate::projects::path_scope::active_project_evidence_roots(&engine, &project.project_id)
            .unwrap(),
        vec![fs::canonicalize(root.join("empty-source")).unwrap()]
    );

    let summary = refresh_active_knowledge_sources_at_startup(
        &engine,
        &knowledge,
        crate::gemma::GemmaService::new_loading(),
    )
    .unwrap();
    assert_eq!(
        summary,
        StartupKnowledgeRefresh {
            refreshed: 1,
            empty: 1,
            failed: 0,
        }
    );
    let refreshed = source_by_id(
        &engine.open_connection().unwrap(),
        &project.project_id,
        &source.source_id,
    )
    .unwrap();
    assert_eq!(refreshed.indexing_state, "ready");
    assert_eq!(refreshed.file_count, 0);
    assert!(refreshed.failure_code.is_none());
    let _ = fs::remove_dir_all(root);
}
