use super::*;
use std::sync::TryLockError;

static JOB: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub(super) fn startup_cancellation() -> &'static AtomicBool {
    &SHUTTING_DOWN
}

pub(super) fn should_schedule(route_tier: &str, model_id: &str) -> bool {
    route_tier == "cloud_tier_2" && !model_id.trim().is_empty()
}

pub(super) fn schedule(model_id: String, model_root: PathBuf) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let Ok(mut job) = JOB.get_or_init(|| Mutex::new(None)).lock() else {
        return;
    };
    if job.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return;
    }
    if let Some(finished) = job.take() {
        let _ = finished.join();
    }
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    match thread::Builder::new()
        .name("oomu-dynamic-local-prewarm".to_string())
        .spawn(move || run(model_id, model_root))
    {
        Ok(handle) => *job = Some(handle),
        Err(error) => eprintln!("DYNAMIC_LOCAL_PREWARM_SPAWN_FAILED message={error}"),
    }
}

pub(super) fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    let job = JOB
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut job| job.take());
    if let Some(job) = job {
        let _ = job.join();
    }
}

fn run(model_id: String, model_root: PathBuf) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        prewarm_if_idle(&model_id, &model_root)
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.code == "local_inference_startup_cancelled" => {}
        Ok(Err(error)) => eprintln!(
            "DYNAMIC_LOCAL_PREWARM_FAILED model_id={} code={} boundary={} message={}",
            crate::redaction::redacted_log_text(&model_id),
            error.code,
            error.boundary,
            crate::redaction::redacted_log_text(&error.message)
        ),
        Err(_) => eprintln!(
            "DYNAMIC_LOCAL_PREWARM_PANICKED model_id={}",
            crate::redaction::redacted_log_text(&model_id)
        ),
    }
}

fn prewarm_if_idle(model_id: &str, model_root: &Path) -> Result<(), InferenceError> {
    ensure_local_infer_idle_reaper();
    let worker = LOCAL_INFER_WORKER.get_or_init(|| Mutex::new(None));
    let mut worker = match worker.try_lock() {
        Ok(worker) => worker,
        Err(TryLockError::WouldBlock) => return Ok(()),
        Err(TryLockError::Poisoned(_)) => {
            return Err(InferenceError::worker(
                "Local inference worker lock was poisoned.",
            ))
        }
    };
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Ok(());
    }
    let requires_restart = worker
        .as_ref()
        .is_none_or(|active| active.model_id != model_id || active.model_root != model_root);
    if requires_restart {
        worker.take();
        *worker = Some(LocalInferWorker::start(
            model_id,
            model_root.to_path_buf(),
            Some(startup_cancellation()),
            None,
        )?);
    }
    if let Some(worker) = worker.as_mut() {
        worker.last_used_at = Instant::now();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cloud_routes_with_a_resolved_baseline_are_eligible() {
        assert!(should_schedule("cloud_tier_2", "gemma-4-12b"));
        assert!(!should_schedule("local_tier_1", "gemma-4-12b"));
        assert!(!should_schedule("cloud_tier_2", " "));
    }
}
