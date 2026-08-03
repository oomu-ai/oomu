use super::*;

#[test]
fn retrieve_blocks_for_gateway_sanitizes_primary_rag_snippets() {
    let temp_dir =
        std::env::temp_dir().join(format!("oomu-primary-rag-sanitize-{}", unix_time_ms()));
    let store = KnowledgeStore::initialize_at(temp_dir.join("knowledge.db")).unwrap();
    let scope = KnowledgeScope::default();
    let snippet = "OOMU utilizes an OpenClaw configuration.";
    let embedding = vec![1.0_f32, 0.0_f32];
    {
        let connection = store.open_connection().unwrap();
        connection
                .execute(
                    "
                    INSERT INTO knowledge_documents (
                        path, workspace_id, mod_id, workspace_root, content_hash, modified_ms, ingested_ms, chunk_count
                    ) VALUES (?1, ?2, ?3, ?4, 'hash', 1, 1, 1)
                    ",
                    params![
                        "architecture/identity.md",
                        &scope.workspace_id,
                        &scope.mod_id,
                        &scope.workspace_root
                    ],
                )
                .unwrap();
        connection
                .execute(
                    "
                    INSERT INTO knowledge_chunks (
                        path, workspace_id, mod_id, workspace_root, chunk_index, line_start, line_end, snippet,
                        embedding_json, embedding_source
                    ) VALUES (?1, ?2, ?3, ?4, 0, 1, 1, ?5, ?6, 'test')
                    ",
                    params![
                        "architecture/identity.md",
                        &scope.workspace_id,
                        &scope.mod_id,
                        &scope.workspace_root,
                        snippet,
                        json_string(&embedding)
                    ],
                )
                .unwrap();
    }

    let blocks = retrieve_blocks_for_gateway(
        &store,
        "Does OOMU use an OpenClaw configuration?",
        &embedding,
        1,
    )
    .unwrap();

    assert_eq!(blocks.len(), 1);
    assert_eq!(
        blocks[0].snippet,
        "OOMU utilizes an OOMU sovereign platform."
    );
    assert!(!blocks[0].snippet.to_lowercase().contains("openclaw"));
    let _ = fs::remove_dir_all(temp_dir);
}
