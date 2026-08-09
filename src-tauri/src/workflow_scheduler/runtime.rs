use std::{
    sync::{mpsc, Mutex, OnceLock},
    thread::{self, JoinHandle},
    time::Duration,
};

const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

enum SchedulerControl {
    Wake,
    Stop,
}

static CURRENT_SCHEDULER: OnceLock<Mutex<Option<mpsc::Sender<SchedulerControl>>>> = OnceLock::new();

pub(crate) struct WorkflowSchedulerRuntime {
    control: mpsc::Sender<SchedulerControl>,
    worker: Mutex<Option<(JoinHandle<()>, mpsc::Receiver<()>)>>,
}

impl WorkflowSchedulerRuntime {
    pub(super) fn spawn(
        worker_name: &str,
        poll_interval: Duration,
        mut tick: impl FnMut() -> bool + Send + 'static,
    ) -> Result<Self, String> {
        let (control, control_requests) = mpsc::channel();
        let (completed, completion) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(worker_name.to_string())
            .spawn(move || {
                let mut should_poll = tick();
                loop {
                    let request = if should_poll {
                        control_requests.recv_timeout(poll_interval)
                    } else {
                        control_requests
                            .recv()
                            .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                    };
                    match request {
                        Ok(SchedulerControl::Wake) | Err(mpsc::RecvTimeoutError::Timeout) => {
                            should_poll = tick();
                        }
                        Ok(SchedulerControl::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break
                        }
                    }
                }
                let _ = completed.send(());
            })
            .map_err(|error| error.to_string())?;
        *CURRENT_SCHEDULER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "workflow_scheduler_control_lock_failed".to_string())? =
            Some(control.clone());
        Ok(Self {
            control,
            worker: Mutex::new(Some((handle, completion))),
        })
    }

    pub(super) fn wake_current() {
        let Some(current) = CURRENT_SCHEDULER.get() else {
            return;
        };
        if let Ok(current) = current.lock() {
            if let Some(control) = current.as_ref() {
                let _ = control.send(SchedulerControl::Wake);
            }
        }
    }

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let _ = self.control.send(SchedulerControl::Stop);
        let mut worker = self
            .worker
            .lock()
            .map_err(|_| "workflow_scheduler_shutdown_lock_failed".to_string())?;
        let Some((handle, completion)) = worker.take() else {
            return Ok(());
        };
        match completion.recv_timeout(SHUTDOWN_WAIT) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => handle
                .join()
                .map_err(|_| "workflow_scheduler_join_panicked".to_string()),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                *worker = Some((handle, completion));
                Err("workflow_scheduler_join_timeout".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowSchedulerRuntime;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[test]
    fn sprint_304_scheduler_stop_wakes_and_joins_the_owned_worker() {
        let ticks = Arc::new(Mutex::new(0_u8));
        let worker_ticks = Arc::clone(&ticks);
        let runtime = WorkflowSchedulerRuntime::spawn(
            "sprint-304-scheduler",
            Duration::from_secs(60),
            move || {
                *worker_ticks.lock().unwrap() += 1;
                false
            },
        )
        .unwrap();

        runtime.shutdown().unwrap();
        runtime.shutdown().unwrap();
        assert_eq!(*ticks.lock().unwrap(), 1);
    }

    #[test]
    fn idle_scheduler_waits_until_work_is_announced() {
        let (ticks, observed) = std::sync::mpsc::channel();
        let runtime = WorkflowSchedulerRuntime::spawn(
            "idle-scheduler",
            Duration::from_millis(10),
            move || {
                ticks.send(()).unwrap();
                false
            },
        )
        .unwrap();

        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(observed.recv_timeout(Duration::from_millis(40)).is_err());
        WorkflowSchedulerRuntime::wake_current();
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        runtime.shutdown().unwrap();
    }
}
