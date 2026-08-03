use std::sync::{Mutex, MutexGuard, OnceLock};

static EXACT_PACKAGE_PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactPackageSurface {
    Presentation,
    Workbook,
}

impl ExactPackageSurface {
    fn label(self) -> &'static str {
        match self {
            Self::Presentation => "Presentation",
            Self::Workbook => "Workbook",
        }
    }
}

/// Holds the one process-wide lease shared by every exact-package renderer
/// caller. The installed macOS renderer initializes the application runtime
/// even for headless conversions, so presentation and workbook launches must
/// never race process registration.
pub(super) fn acquire_exact_package_process(
    surface: ExactPackageSurface,
) -> Result<MutexGuard<'static, ()>, String> {
    EXACT_PACKAGE_PROCESS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            format!(
                "{} converter serialization is unavailable.",
                surface.label()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn presentation_and_workbook_share_one_exact_package_process_lease() {
        let presentation =
            acquire_exact_package_process(ExactPackageSurface::Presentation).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();

        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let started = Instant::now();
            let workbook = acquire_exact_package_process(ExactPackageSurface::Workbook).unwrap();
            acquired_tx.send(started.elapsed()).unwrap();
            drop(workbook);
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        drop(presentation);

        let blocked_for = acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(blocked_for >= Duration::from_millis(100));
        waiter.join().unwrap();
    }
}
