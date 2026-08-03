use std::{
    sync::{mpsc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

const SHUTDOWN_WAIT: Duration = Duration::from_secs(5);

pub(crate) struct WorkflowSchedulerRuntime {
    stop: mpsc::Sender<()>,
    worker: Mutex<Option<(JoinHandle<()>, mpsc::Receiver<()>)>>,
}

impl WorkflowSchedulerRuntime {
    pub(super) fn spawn(
        worker_name: &str,
        poll_interval: Duration,
        mut tick: impl FnMut() + Send + 'static,
    ) -> Result<Self, String> {
        let (stop, stop_requests) = mpsc::channel();
        let (completed, completion) = mpsc::channel();
        let handle = thread::Builder::new()
            .name(worker_name.to_string())
            .spawn(move || {
                loop {
                    tick();
                    match stop_requests.recv_timeout(poll_interval) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                let _ = completed.send(());
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            worker: Mutex::new(Some((handle, completion))),
        })
    }

    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let _ = self.stop.send(());
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
            move || *worker_ticks.lock().unwrap() += 1,
        )
        .unwrap();

        runtime.shutdown().unwrap();
        runtime.shutdown().unwrap();
        assert_eq!(*ticks.lock().unwrap(), 1);
    }
}
