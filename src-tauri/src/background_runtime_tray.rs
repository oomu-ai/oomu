use crate::{db, routines, settings, startup_splash};
use tauri::Manager;

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

#[cfg(target_os = "macos")]
const TRAY_ID: &str = "oomu-background";
#[cfg(target_os = "macos")]
const OPEN_ID: &str = "oomu-background-open";
#[cfg(target_os = "macos")]
const STATUS_ID: &str = "oomu-background-status";
#[cfg(target_os = "macos")]
const QUIT_ID: &str = "oomu-background-quit";

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayCopy {
    running: String,
    open: String,
    quit: String,
}

#[cfg(target_os = "macos")]
impl TrayCopy {
    pub(crate) fn new(
        running: impl Into<String>,
        open: impl Into<String>,
        quit: impl Into<String>,
    ) -> Self {
        Self {
            running: running.into(),
            open: open.into(),
            quit: quit.into(),
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn tray_copy(translations: Option<&serde_json::Value>) -> Result<TrayCopy, String> {
    let translations =
        translations.ok_or_else(|| "background_menu_language_unavailable".to_string())?;
    let value = |key: &str| {
        translations
            .pointer(&format!("/menu_bar/{key}"))
            .and_then(serde_json::Value::as_str)
            .filter(|copy| !copy.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| "background_menu_language_unavailable".to_string())
    };
    Ok(TrayCopy::new(
        value("background_running")?,
        value("open_oomu")?,
        value("quit_oomu")?,
    ))
}

#[cfg(target_os = "macos")]
fn tray_menu(app: &tauri::AppHandle, copy: &TrayCopy) -> tauri::Result<Menu<tauri::Wry>> {
    let status = MenuItem::with_id(app, STATUS_ID, &copy.running, false, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN_ID, &copy.open, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_ID, &copy.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&status, &open, &separator, &quit])
}

#[cfg(target_os = "macos")]
pub(crate) fn restore_foreground(app: &tauri::AppHandle) {
    if !startup_splash::main_window_reveal_allowed() {
        startup_splash::refocus_active();
        return;
    }
    if let Some(engine) = app.try_state::<db::PersistenceEngine>() {
        routines::background::record_window_reopened(engine.inner());
    }
    if let Err(error) = app.set_activation_policy(tauri::ActivationPolicy::Regular) {
        eprintln!("OOMU_FOREGROUND_RESTORE_POLICY_FAILED error={error}");
    }
    if let Err(error) = app.show() {
        eprintln!("OOMU_FOREGROUND_RESTORE_APP_FAILED error={error}");
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        if let Err(error) = window.show() {
            eprintln!("OOMU_FOREGROUND_RESTORE_WINDOW_FAILED error={error}");
        }
        if let Err(error) = window.set_focus() {
            eprintln!("OOMU_FOREGROUND_RESTORE_FOCUS_FAILED error={error}");
        }
    }
}

#[cfg(target_os = "macos")]
fn create_tray(app: &tauri::AppHandle, persistence: &db::PersistenceEngine) -> Result<(), String> {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let locale = settings::locale_state_for_engine(persistence, None)
        .map_err(|_| "background_menu_language_unavailable".to_string())?;
    let copy = tray_copy(Some(&locale.translations))?;
    let menu = tray_menu(app, &copy).map_err(|error| error.to_string())?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/oomu-menu-raven.png"))
        .map_err(|error| error.to_string())?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip(&copy.running)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                restore_foreground(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => restore_foreground(app),
            QUIT_ID => app.exit(0),
            _ => {}
        })
        .build(app)
        .map_err(|error| error.to_string())?;
    tray.set_visible(true).map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub(crate) fn sync_background_tray(
    app: &tauri::AppHandle,
    persistence: &db::PersistenceEngine,
    visible: bool,
) -> Result<(), String> {
    if visible {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            tray.set_visible(true).map_err(|error| error.to_string())?;
        } else {
            create_tray(app, persistence)?;
        }
    } else if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // Hiding preserves the native event-loop owner while removing every
        // visible claim that background work is active. Dropping the final
        // tray object can terminate a windowless macOS process before its
        // recovery surface is available.
        tray.set_visible(false).map_err(|error| error.to_string())?;
    }
    if let Err(error) = routines::background::record_menu_visibility(persistence, visible) {
        if visible {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                let _ = tray.set_visible(false);
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn sync_background_tray(
    _app: &tauri::AppHandle,
    _persistence: &db::PersistenceEngine,
    _visible: bool,
) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn refresh_background_tray_menu(
    app: &tauri::AppHandle,
    translations: &serde_json::Value,
) -> Result<(), String> {
    let copy = tray_copy(Some(translations))?;
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return Ok(());
    };
    let menu = tray_menu(app, &copy).map_err(|error| error.to_string())?;
    tray.set_menu(Some(menu))
        .and_then(|_| tray.set_tooltip(Some(&copy.running)))
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn refresh_background_tray_menu(
    _app: &tauri::AppHandle,
    _translations: &serde_json::Value,
) -> Result<(), String> {
    Ok(())
}
