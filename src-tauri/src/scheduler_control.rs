use std::sync::{mpsc, Mutex, OnceLock};

pub(crate) enum SchedulerControl {
    Wake,
    Stop,
}

static CURRENT_SCHEDULERS: OnceLock<Mutex<Vec<mpsc::Sender<SchedulerControl>>>> = OnceLock::new();

pub(crate) fn register_current(control: mpsc::Sender<SchedulerControl>) -> Result<(), String> {
    CURRENT_SCHEDULERS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "workflow_scheduler_control_lock_failed".to_string())?
        .push(control);
    Ok(())
}

pub(crate) fn wake_current() {
    let Some(current) = CURRENT_SCHEDULERS.get() else {
        return;
    };
    if let Ok(mut current) = current.lock() {
        current.retain(|control| control.send(SchedulerControl::Wake).is_ok());
    }
}
