use super::*;

pub(super) fn ensure_startup_active(
    startup_cancellation: Option<&AtomicBool>,
    stream_id: Option<&str>,
) -> Result<(), InferenceError> {
    ensure_stream_active(stream_id)?;
    if startup_cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        return Err(InferenceError::local_infer(
            "local_inference_startup_cancelled",
            "Local model startup was cancelled during shutdown.",
        ));
    }
    Ok(())
}

pub(super) fn ensure_operation_active(stream_id: Option<&str>) -> Result<(), InferenceError> {
    ensure_stream_active(stream_id)?;
    if local_prewarm::startup_cancellation().load(Ordering::Acquire) {
        return Err(InferenceError::local_infer(
            "local_inference_cancelled",
            "Local inference was cancelled during shutdown.",
        ));
    }
    Ok(())
}

fn ensure_stream_active(stream_id: Option<&str>) -> Result<(), InferenceError> {
    if is_local_stream_cancelled(stream_id) {
        return Err(InferenceError::local_infer(
            "local_inference_cancelled",
            "Local generation was cancelled.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_cancelled_stream_is_rejected_before_worker_start_or_prompt_write() {
        let stream_id = format!(
            "cancel-before-start-{}",
            crate::foundation::clock::unix_time_ns_u128()
        );
        assert!(cancel_chat_stream(stream_id.clone()));
        let error = ensure_operation_active(Some(&stream_id)).expect_err("stream is cancelled");
        assert_eq!(error.code, "local_inference_cancelled");
        clear_local_stream_cancellation(&stream_id);
    }
}
