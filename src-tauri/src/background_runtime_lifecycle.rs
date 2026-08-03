use crate::{db, redaction, routines, sync_background_tray};
use tauri::{Emitter, Manager};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseRequestDisposition {
    Hidden,
    KeepOpen,
    ExitApplication,
}

pub(crate) fn reconcile_startup_handle(app: &tauri::AppHandle) {
    let engine = app.state::<db::PersistenceEngine>().inner().clone();
    let runtime = app
        .state::<routines::BackgroundRuntimeSupervisor>()
        .inner()
        .clone();
    let reconciliation = match routines::background::begin_startup_reconciliation(&engine) {
        Ok(reconciliation) => reconciliation,
        Err(error) => {
            eprintln!(
                "OOMU_BACKGROUND_RECONCILIATION_FAILED {}",
                redaction::redacted_log_text(&error)
            );
            return;
        }
    };
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = routines::background::finish_startup_reconciliation(
            app_handle.clone(),
            &engine,
            &runtime,
            &reconciliation,
        );
        let publish_app = app_handle.clone();
        if let Err(error) = app_handle.run_on_main_thread(move || match result {
            Ok(_) => publish_reconciled_status(&publish_app, &engine),
            Err(error) => eprintln!(
                "OOMU_BACKGROUND_RECONCILIATION_FAILED {}",
                redaction::redacted_log_text(&error)
            ),
        }) {
            eprintln!("OOMU_BACKGROUND_RECONCILIATION_PUBLISH_FAILED {error}");
        }
    });
}

fn publish_reconciled_status(app: &tauri::AppHandle, engine: &db::PersistenceEngine) {
    let tray_visible = routines::background::menu_should_be_visible(engine).unwrap_or(false);
    if let Err(error) = sync_background_tray(app, engine, tray_visible) {
        eprintln!(
            "OOMU_BACKGROUND_TRAY_SYNC_FAILED {}",
            redaction::redacted_log_text(&error)
        );
        routines::background::record_runtime_attention(engine, "background_menu_evidence_failed");
    }
    if let Ok(status) = routines::background::status(engine) {
        let _ = app.emit(
            routines::background::BACKGROUND_RUNTIME_STATUS_EVENT,
            status,
        );
    }
}

pub(crate) fn hide_main_window_if_verified(window: &tauri::Window) -> CloseRequestDisposition {
    let engine = window.state::<db::PersistenceEngine>();
    if !routines::should_keep_alive(engine.inner()) {
        return CloseRequestDisposition::ExitApplication;
    }
    let result = transition_to_background(
        || window.hide().map_err(|error| error.to_string()),
        || set_accessory_policy(window.app_handle()),
        || restore_visible_regular(window),
    );
    if let Err(code) = result {
        routines::background::record_runtime_attention(engine.inner(), code);
        eprintln!("OOMU_BACKGROUND_WINDOW_TRANSITION_FAILED code={code}");
        return CloseRequestDisposition::KeepOpen;
    }
    routines::background::record_window_closed(engine.inner());
    CloseRequestDisposition::Hidden
}

fn transition_to_background(
    hide: impl FnOnce() -> Result<(), String>,
    set_accessory: impl FnOnce() -> Result<(), String>,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<(), &'static str> {
    if hide().is_err() {
        return match restore() {
            Ok(()) => Err("background_window_hide_failed"),
            Err(_) => Err("background_window_restore_failed"),
        };
    }
    if set_accessory().is_err() {
        return match restore() {
            Ok(()) => Err("background_window_policy_failed"),
            Err(_) => Err("background_window_restore_failed"),
        };
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_accessory_policy(app: &tauri::AppHandle) -> Result<(), String> {
    app.set_activation_policy(tauri::ActivationPolicy::Accessory)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
fn set_accessory_policy(_app: &tauri::AppHandle) -> Result<(), String> {
    Ok(())
}

fn restore_visible_regular(window: &tauri::Window) -> Result<(), String> {
    let mut first_failure = None;
    let mut record = |result: tauri::Result<()>| {
        if let Err(error) = result {
            first_failure.get_or_insert_with(|| error.to_string());
        }
    };
    #[cfg(target_os = "macos")]
    record(
        window
            .app_handle()
            .set_activation_policy(tauri::ActivationPolicy::Regular),
    );
    record(window.app_handle().show());
    record(window.show());
    record(window.unminimize());
    record(window.set_focus());
    first_failure.map_or(Ok(()), Err)
}

pub(crate) fn stop_background_runtime(app: &tauri::AppHandle) -> Result<(), String> {
    let (Some(engine), Some(supervisor)) = (
        app.try_state::<db::PersistenceEngine>(),
        app.try_state::<routines::BackgroundRuntimeSupervisor>(),
    ) else {
        return Ok(());
    };
    routines::background::prepare_explicit_quit(engine.inner(), supervisor.inner())
}

pub(crate) fn remove_background_ui(app: &tauri::AppHandle) -> Result<(), String> {
    let Some(engine) = app.try_state::<db::PersistenceEngine>() else {
        return Ok(());
    };
    sync_background_tray(app, engine.inner(), false)
}

pub(crate) fn reveal_recovery_without_focus(app: &tauri::AppHandle) -> Result<(), String> {
    let mut first_failure = None;
    let mut record = |result: tauri::Result<()>| {
        if let Err(error) = result {
            first_failure.get_or_insert_with(|| error.to_string());
        }
    };
    #[cfg(target_os = "macos")]
    record(app.set_activation_policy(tauri::ActivationPolicy::Regular));
    record(app.show());
    if let Some(window) = app.get_webview_window("main") {
        record(window.show());
        record(window.unminimize());
    }
    first_failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::transition_to_background;
    use std::cell::Cell;

    #[test]
    fn hide_failure_restores_foreground_without_claiming_background_success() {
        let accessory_calls = Cell::new(0);
        let restore_calls = Cell::new(0);
        let result = transition_to_background(
            || Err("hide failed".to_string()),
            || {
                accessory_calls.set(accessory_calls.get() + 1);
                Ok(())
            },
            || {
                restore_calls.set(restore_calls.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result, Err("background_window_hide_failed"));
        assert_eq!(accessory_calls.get(), 0);
        assert_eq!(restore_calls.get(), 1);
    }
}
