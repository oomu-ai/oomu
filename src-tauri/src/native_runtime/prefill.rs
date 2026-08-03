use super::*;

const MAX_CANCELLABLE_PREFILL_BATCH_TOKENS: usize = 64;

pub(super) fn decode_tokens(
    context: &mut LlamaContext<'_>,
    sequence_id: i32,
    start_position: usize,
    tokens: &[LlamaToken],
    batch_size: usize,
    cancellation: Option<&AtomicBool>,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<(), NativeRuntimeError> {
    ensure_active(cancellation)?;
    let chunk_size = cancellable_batch_size(batch_size);
    let mut evaluated = 0usize;
    for chunk in tokens.chunks(chunk_size) {
        ensure_active(cancellation)?;
        let mut batch = LlamaBatch::new(chunk.len(), 1);
        let chunk_start = start_position + evaluated;
        for (offset, token) in chunk.iter().enumerate() {
            let position = i32::try_from(chunk_start + offset).map_err(|_| NativeRuntimeError {
                code: "llama_context_position_overflow",
                message: "The llama.cpp context position exceeded its supported range.".to_string(),
            })?;
            batch
                .add(*token, position, &[sequence_id], offset + 1 == chunk.len())
                .map_err(|error| NativeRuntimeError {
                    code: "llama_batch_build_failed",
                    message: format!("Unable to build the llama.cpp decode batch: {error}"),
                })?;
        }
        context
            .decode(&mut batch)
            .map_err(|error| NativeRuntimeError {
                code: "llama_context_decode_failed",
                message: format!("llama.cpp failed to append tokens to the KV cache: {error}"),
            })?;
        evaluated += chunk.len();
        on_progress(evaluated, tokens.len());
        ensure_active(cancellation)?;
    }
    Ok(())
}

pub(super) fn ensure_active(cancellation: Option<&AtomicBool>) -> Result<(), NativeRuntimeError> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(NativeRuntimeError {
            code: "local_inference_cancelled",
            message: "Local inference was cancelled during prompt prefill.".to_string(),
        });
    }
    Ok(())
}

pub(super) fn invalidate_failed_prefill(session: &mut SessionCache) {
    session.source_tokens.clone_from(&session.tokens);
    session.pinned_tokens = session.pinned_tokens.min(session.tokens.len());
    session.resident = false;
    session.last_used = Instant::now();
}

pub(super) fn cancelled_generation_result(session_id: &str) -> NativeGenerationResult {
    NativeGenerationResult {
        text: String::new(),
        raw_text: String::new(),
        token_ids: Vec::new(),
        time_to_first_token_ms: 0,
        cancelled: true,
        session_stats: NativeSessionStats {
            session_id: normalize_session_id(session_id),
            cached_tokens: 0,
            evaluated_tokens: 0,
            context_tokens: 0,
            pinned_tokens: 0,
            shifted_tokens: 0,
            evicted_sessions: 0,
            cold_start: true,
        },
    }
}

pub(super) fn progress_event(sequence: usize, elapsed_ms: u128) -> NativeTokenEvent {
    NativeTokenEvent {
        sequence,
        token_id: -1,
        text: String::new(),
        elapsed_ms,
    }
}

pub(super) fn decode_generated_token(
    context: &mut LlamaContext<'_>,
    sequence_id: i32,
    position: usize,
    token: LlamaToken,
) -> Result<(), NativeRuntimeError> {
    decode_tokens(context, sequence_id, position, &[token], 1, None, |_, _| {})
}

fn cancellable_batch_size(configured: usize) -> usize {
    configured.max(1).min(MAX_CANCELLABLE_PREFILL_BATCH_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefill_batches_are_bounded_for_progress_and_cancellation() {
        assert_eq!(cancellable_batch_size(0), 1);
        assert_eq!(cancellable_batch_size(32), 32);
        assert_eq!(cancellable_batch_size(512), 64);
    }

    #[test]
    fn pre_cancelled_prefill_fails_with_the_stable_cancellation_code() {
        let cancellation = AtomicBool::new(true);
        let error = ensure_active(Some(&cancellation)).expect_err("prefill must cancel");
        assert_eq!(error.code, "local_inference_cancelled");

        let result = cancelled_generation_result(" session ");
        assert!(result.cancelled);
        assert!(result.token_ids.is_empty());
        assert_eq!(result.session_stats.session_id, "session");
    }

    #[test]
    fn failed_prefill_invalidates_every_logical_cache_claim() {
        let target = vec![
            LlamaToken::new(1),
            LlamaToken::new(2),
            LlamaToken::new(3),
            LlamaToken::new(4),
        ];
        let mut session = SessionCache {
            sequence_id: 0,
            tokens: target[..2].to_vec(),
            source_tokens: target.clone(),
            pinned_tokens: 3,
            system_prompt: Some("system".to_string()),
            resident: true,
            last_used: Instant::now(),
        };

        invalidate_failed_prefill(&mut session);

        assert_eq!(session.source_tokens, session.tokens);
        assert_eq!(session.pinned_tokens, session.tokens.len());
        assert!(!session.resident);
        let mut retry_tokens = session.tokens.clone();
        retry_tokens.extend_from_slice(&target[session.source_tokens.len()..]);
        assert_eq!(retry_tokens, target);
    }
}
