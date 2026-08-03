use std::sync::atomic::{AtomicBool, Ordering};

static INTERRUPTION_USED: AtomicBool = AtomicBool::new(false);

pub(super) fn maybe_interrupt(
    gemma: &crate::gemma::GemmaService,
) -> Result<(), super::InferenceError> {
    let requested = crate::launch_startup::sprint_294_isolated_profile::is_active()
        && std::env::var("OOMU_SPRINT_300_RECOVERABLE_CLASSIFIER_INTERRUPT").as_deref() == Ok("1")
        && !INTERRUPTION_USED.swap(true, Ordering::SeqCst);
    if !requested {
        return Ok(());
    }
    let code = "classifier_qualification_interrupted";
    let boundary = "auto_route_classifier_qualification";
    let message =
        "The test-owned classifier interruption paused this turn before provider dispatch.";
    gemma.mark_classifier_failure(code, boundary, message);
    Err(super::InferenceError::routing_attention(
        code, boundary, message,
    ))
}
