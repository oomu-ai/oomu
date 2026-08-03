use super::*;

#[test]
fn ingest_request_rejects_renderer_paths_and_workspace_roots() {
    let raw_paths = serde_json::json!({
        "grantId": random_grant_id(),
        "sessionId": "session-a",
        "turnId": "turn-a",
        "paths": ["/etc/passwd"]
    });
    let raw_root = serde_json::json!({
        "grantId": random_grant_id(),
        "sessionId": "session-a",
        "turnId": "turn-a",
        "workspaceRoot": "/"
    });

    assert!(serde_json::from_value::<KnowledgeIngestRequest>(raw_paths).is_err());
    assert!(serde_json::from_value::<KnowledgeIngestRequest>(raw_root).is_err());
}

#[test]
fn knowledge_document_keys_are_logical_not_filesystem_paths() {
    assert!(normalize_document_key("folder/guide.md").is_ok());
    assert!(normalize_document_key("/etc/passwd").is_err());
    assert!(normalize_document_key("../secret.md").is_err());
}

#[test]
fn sliding_chunks_preserve_line_numbers_with_overlap() {
    let content = (1..=95)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let chunks = sliding_chunks(&content);

    assert_eq!(chunks.len(), 2);
    assert_eq!((chunks[0].0, chunks[0].1), (1, 80));
    assert_eq!((chunks[1].0, chunks[1].1), (69, 95));
}

#[test]
fn select_chunks_applies_mod_and_workspace_prefilter() {
    let temp_dir = std::env::temp_dir().join(format!("oomu-knowledge-scope-{}", unix_time_ms()));
    fs::create_dir_all(&temp_dir).unwrap();
    let store = KnowledgeStore::initialize_at(temp_dir.join("knowledge.db")).unwrap();
    let connection = store.open_connection().unwrap();
    let workspace_a = workspace_id_for_root("/workspace/a");
    let workspace_b = workspace_id_for_root("/workspace/b");
    connection
            .execute(
                "
                INSERT INTO knowledge_documents (
                    path, workspace_id, mod_id, workspace_root, content_hash, modified_ms, ingested_ms, chunk_count
                ) VALUES (?1, ?2, ?3, ?4, 'hash-a', 1, 1, 1)
                ",
                params!["src/a.rs", &workspace_a, "module-a", "/workspace/a"],
            )
            .unwrap();
    connection
            .execute(
                "
                INSERT INTO knowledge_documents (
                    path, workspace_id, mod_id, workspace_root, content_hash, modified_ms, ingested_ms, chunk_count
                ) VALUES (?1, ?2, ?3, ?4, 'hash-b', 1, 1, 1)
                ",
                params!["src/b.rs", &workspace_b, "module-b", "/workspace/b"],
            )
            .unwrap();
    connection
            .execute(
                "
                INSERT INTO knowledge_chunks (
                    path, workspace_id, mod_id, workspace_root, chunk_index, line_start, line_end, snippet,
                    embedding_json, embedding_source
                ) VALUES (?1, ?2, ?3, ?4, 0, 1, 1, 'target snippet', '[1.0,0.0]', 'test')
                ",
                params!["src/a.rs", &workspace_a, "module-a", "/workspace/a"],
            )
            .unwrap();
    connection
            .execute(
                "
                INSERT INTO knowledge_chunks (
                    path, workspace_id, mod_id, workspace_root, chunk_index, line_start, line_end, snippet,
                    embedding_json, embedding_source
                ) VALUES (?1, ?2, ?3, ?4, 0, 1, 1, 'other snippet', '[0.0,1.0]', 'test')
                ",
                params!["src/b.rs", &workspace_b, "module-b", "/workspace/b"],
            )
            .unwrap();
    drop(connection);

    let chunks = store
        .select_chunks(&KnowledgeScope {
            workspace_id: workspace_a,
            mod_id: "module-a".to_string(),
            workspace_root: "/workspace/a".to_string(),
            project_id: None,
        })
        .unwrap();

    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].path, "src/a.rs");
    assert_eq!(chunks[0].snippet, "target snippet");

    let _ = fs::remove_dir_all(temp_dir);
}
