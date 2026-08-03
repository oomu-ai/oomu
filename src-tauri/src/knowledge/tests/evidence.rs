use super::*;

#[test]
fn native_grant_reads_exact_files_once() {
    let root = temp_knowledge_directory("grant-once");
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/guide.md"), "exact native grant content").unwrap();
    let store = KnowledgeIngestGrantStore::default();
    let issued =
        issue_knowledge_ingest_grant(&store, &root, "session-a", "turn-a", None, 10).unwrap();
    let request = KnowledgeIngestRequest {
        grant_id: issued.grant_id,
        session_id: "session-a".to_string(),
        turn_id: "turn-a".to_string(),
        mod_id: None,
        project_id: None,
    };

    let files = consume_knowledge_ingest_grant(&store, &request).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].display_path.ends_with("/notes/guide.md"));
    assert_eq!(files[0].content, "exact native grant content");
    let replay = consume_knowledge_ingest_grant(&store, &request).unwrap_err();
    assert_eq!(replay.code, "knowledge_grant_rejected");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_grant_preserves_an_empty_folder_for_future_startup_scans() {
    let root = temp_knowledge_directory("empty-folder");
    fs::create_dir_all(&root).unwrap();
    let store = KnowledgeIngestGrantStore::default();
    let issued =
        issue_knowledge_ingest_grant(&store, &root, "session-a", "turn-a", None, 10).unwrap();
    assert_eq!(issued.file_count, 0);
    assert_eq!(issued.total_bytes, 0);
    let request = KnowledgeIngestRequest {
        grant_id: issued.grant_id,
        session_id: "session-a".to_string(),
        turn_id: "turn-a".to_string(),
        mod_id: None,
        project_id: Some("project_11111111-1111-4111-8111-111111111111".to_string()),
    };
    assert!(consume_knowledge_ingest_grant(&store, &request)
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn knowledge_grant_rejects_scope_mismatch_and_expiry() {
    let root = temp_knowledge_directory("scope-expiry");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("guide.md"), "scoped grant content").unwrap();
    let store = KnowledgeIngestGrantStore::default();
    let mismatch =
        issue_knowledge_ingest_grant(&store, &root, "session-a", "turn-a", None, 10).unwrap();
    let mismatch_request = KnowledgeIngestRequest {
        grant_id: mismatch.grant_id,
        session_id: "session-b".to_string(),
        turn_id: "turn-a".to_string(),
        mod_id: None,
        project_id: None,
    };
    assert!(consume_knowledge_ingest_grant(&store, &mismatch_request).is_err());

    let expired =
        issue_knowledge_ingest_grant(&store, &root, "session-a", "turn-a", None, 10).unwrap();
    store
        .state
        .lock()
        .unwrap()
        .grants
        .get_mut(&expired.grant_id)
        .unwrap()
        .expires_at_ms = unix_time_ms() - 1;
    let expired_request = KnowledgeIngestRequest {
        grant_id: expired.grant_id,
        session_id: "session-a".to_string(),
        turn_id: "turn-a".to_string(),
        mod_id: None,
        project_id: None,
    };
    assert!(consume_knowledge_ingest_grant(&store, &expired_request).is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn knowledge_grant_rejects_file_identity_changes() {
    let root = temp_knowledge_directory("identity-change");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("guide.md");
    fs::write(&file, "original knowledge").unwrap();
    let store = KnowledgeIngestGrantStore::default();
    let issued =
        issue_knowledge_ingest_grant(&store, &root, "session-a", "turn-a", None, 10).unwrap();
    fs::write(&file, "changed knowledge with a different size").unwrap();
    let request = KnowledgeIngestRequest {
        grant_id: issued.grant_id,
        session_id: "session-a".to_string(),
        turn_id: "turn-a".to_string(),
        mod_id: None,
        project_id: None,
    };

    assert!(consume_knowledge_ingest_grant(&store, &request).is_err());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn knowledge_picker_rejects_symlink_files() {
    use std::os::unix::fs::symlink;

    let root = temp_knowledge_directory("symlink");
    let outside = temp_knowledge_directory("symlink-outside");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret.md"), "outside secret").unwrap();
    symlink(outside.join("secret.md"), root.join("linked.md")).unwrap();

    let error = issue_knowledge_ingest_grant(
        &KnowledgeIngestGrantStore::default(),
        &root,
        "session-a",
        "turn-a",
        None,
        10,
    )
    .unwrap_err();
    assert_eq!(error.code, "knowledge_grant_rejected");
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn source_tagged_prompt_marks_audit_sources() {
    let prompt = source_tagged_prompt(
        "Where is sync_provider_models defined?",
        &[KnowledgeContextBlock {
            path: "src-tauri/src/inference/mod.rs".to_string(),
            line_start: 160,
            line_end: 220,
            score: 0.91,
            semantic_relevance_score: 0.82,
            lexical_relevance_score: 0.09,
            overlap_percent: 15,
            token_count: 8,
            snippet: "pub async fn sync_provider_models(".to_string(),
        }],
    );

    assert!(prompt.contains("[SOURCE] src-tauri/src/inference/mod.rs:160-220"));
    assert!(prompt.contains("User request:"));
}

#[test]
fn sanitize_competitor_terms_rewrites_identity_leak_phrases() {
    let cleansed = sanitize_competitor_terms(
        "OOMU utilizes an OpenClaw configuration. This Open-Claw wrapper mentions openclaw.",
    );

    assert!(cleansed.contains("OOMU utilizes an OOMU sovereign platform."));
    assert!(cleansed.contains("OOMU high-performance kernel"));
    assert!(cleansed.contains("OOMU (custom local-first runtime)"));
    assert!(!cleansed.to_lowercase().contains("openclaw"));
    assert!(!cleansed.to_lowercase().contains("open-claw"));
}

#[test]
fn verify_mod_rag_isolation() {
    let mod_id = format!("ai.eldris.mods.cs-{}", unix_time_ms());
    let db_path = crate::db::get_mod_db_path(&mod_id).unwrap();
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let snippet = "EXCEL-CS-7798: Unplug machine for 30s to reset memory.";
    let embedding = vec![1.0_f32, 0.0_f32];
    {
        let connection = Connection::open(&db_path).unwrap();
        connection
            .execute_batch(
                "
                    CREATE TABLE knowledge_documents (
                        path TEXT PRIMARY KEY,
                        content_hash TEXT NOT NULL,
                        modified_ms INTEGER NOT NULL,
                        ingested_ms INTEGER NOT NULL,
                        chunk_count INTEGER NOT NULL
                    );
                    CREATE TABLE knowledge_chunks (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        path TEXT NOT NULL,
                        chunk_index INTEGER NOT NULL,
                        line_start INTEGER NOT NULL,
                        line_end INTEGER NOT NULL,
                        snippet TEXT NOT NULL,
                        embedding_json TEXT NOT NULL,
                        embedding_source TEXT NOT NULL
                    );
                    ",
            )
            .unwrap();
        connection
            .execute(
                "
                    INSERT INTO knowledge_documents (
                        path, content_hash, modified_ms, ingested_ms, chunk_count
                    ) VALUES ('cs/reset.md', 'hash', 1, 1, 1)
                    ",
                [],
            )
            .unwrap();
        connection
                .execute(
                    "
                    INSERT INTO knowledge_chunks (
                        path, chunk_index, line_start, line_end, snippet, embedding_json, embedding_source
                    ) VALUES (?1, 0, 1, 1, ?2, ?3, 'test')
                    ",
                    params![
                        "cs/reset.md",
                        snippet,
                        json_string(&embedding)
                    ],
                )
                .unwrap();
    }

    let oomu_contexts = retrieve_mod_blocks_for_gateway(
        "How do I reset machine memory?",
        &embedding,
        &[mod_id.clone()],
        3,
    )
    .unwrap();
    let oomu_prompt = mod_source_tagged_context(&oomu_contexts).unwrap();
    assert!(oomu_prompt.contains(&format!(
        "[MOD KNOWLEDGE BASE RETRIEVAL - SOURCE: {mod_id}]"
    )));
    assert!(oomu_prompt.contains("EXCEL-CS-7798"));
    assert!(oomu_prompt.contains("[END MOD KNOWLEDGE BASE RETRIEVAL]"));

    let temp_dir = std::env::temp_dir().join(format!("oomu-primary-rag-empty-{}", unix_time_ms()));
    let primary_store = KnowledgeStore::initialize_at(temp_dir.join("knowledge.db")).unwrap();
    let oomu_blocks = retrieve_blocks_for_gateway(
        &primary_store,
        "How do I reset machine memory?",
        &embedding,
        3,
    )
    .unwrap();
    let oomu_prompt = source_tagged_context(&oomu_blocks).unwrap_or_default();
    assert!(!oomu_prompt.contains("EXCEL-CS-7798"));

    let mod_root = db_path.parent().unwrap().parent().unwrap().to_path_buf();
    let _ = fs::remove_dir_all(mod_root);
    let _ = fs::remove_dir_all(temp_dir);
}
