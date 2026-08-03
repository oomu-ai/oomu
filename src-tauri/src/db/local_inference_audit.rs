use super::*;

impl PersistenceEngine {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_local_inference_audit(
        &self,
        event_kind: &str,
        prompt: &str,
        output: &str,
        trace_hash: &str,
        device: &str,
        latency_ms: u128,
        time_to_first_token_ms: u128,
        prompt_token_count: usize,
        generated_token_count: usize,
    ) -> rusqlite::Result<()> {
        self.insert_local_inference_audit_hashes(
            event_kind,
            &sha256_hex(prompt.as_bytes()),
            &sha256_hex(output.as_bytes()),
            trace_hash,
            device,
            latency_ms,
            time_to_first_token_ms,
            prompt_token_count,
            generated_token_count,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn insert_local_inference_audit_hashes(
        &self,
        event_kind: &str,
        prompt_hash: &str,
        output_hash: &str,
        trace_hash: &str,
        device: &str,
        latency_ms: u128,
        time_to_first_token_ms: u128,
        prompt_token_count: usize,
        generated_token_count: usize,
    ) -> rusqlite::Result<()> {
        let _guard = self.lock_writes();
        let connection = self.open_ops_connection()?;
        ensure_local_inference_audit_schema(&connection)?;
        let deterministic_transform = event_kind
            == crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_EVENT_KIND;
        let metadata = json!({
            "model_store": if deterministic_transform {
                crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_MODEL_PATH
            } else {
                "private://local-model/active"
            },
            "execution_kind": if deterministic_transform {
                "native_deterministic_transform"
            } else {
                "transformer_inference"
            },
            "device": device,
            "latency_ms": latency_ms,
            "time_to_first_token_ms": time_to_first_token_ms,
            "input_tokens_estimate": prompt_token_count,
            "output_tokens_estimate": generated_token_count,
        })
        .to_string();
        connection.execute(
            "
            INSERT INTO local_inference_audit (
                event_id, event_kind, prompt_hash, output_hash, trace_hash,
                metadata_json, created_at_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                format!(
                    "local-infer-{}",
                    crate::foundation::clock::unix_time_ns_u128()
                ),
                event_kind,
                prompt_hash,
                output_hash,
                trace_hash,
                metadata,
                unix_time_ms(),
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_rewrite_audit_does_not_claim_transformer_execution() {
        let root = std::env::temp_dir().join(format!(
            "oomu-deterministic-rewrite-audit-{}-{}",
            std::process::id(),
            crate::foundation::clock::unix_time_ns_u128()
        ));
        std::fs::create_dir_all(&root).expect("create audit test directory");
        let engine = PersistenceEngine::initialize_at(root.join("state.sqlite"))
            .expect("initialize audit database");
        engine
            .insert_local_inference_audit_hashes(
                crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_EVENT_KIND,
                "prompt-hash",
                "output-hash",
                "trace-hash",
                crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_DEVICE,
                1,
                0,
                0,
                0,
            )
            .expect("persist deterministic transform audit");

        let connection = engine.open_ops_connection().expect("open audit database");
        let (event_kind, metadata): (String, String) = connection
            .query_row(
                "SELECT event_kind, metadata_json FROM local_inference_audit LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read deterministic transform audit");
        let metadata: serde_json::Value =
            serde_json::from_str(&metadata).expect("audit metadata is JSON");
        assert_eq!(
            event_kind,
            crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_EVENT_KIND
        );
        assert_eq!(metadata["execution_kind"], "native_deterministic_transform");
        assert_eq!(
            metadata["model_store"],
            crate::gemma::deterministic_transform::BOUNDED_REWRITE_TRANSFORM_MODEL_PATH
        );
        assert_eq!(metadata["output_tokens_estimate"], 0);

        let _ = std::fs::remove_dir_all(root);
    }
}
