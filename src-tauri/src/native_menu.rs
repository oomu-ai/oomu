use crate::{app_updates::ApplicationUpdateService, background_runtime_tray};
use tauri::{
    menu::{
        AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
        WINDOW_SUBMENU_ID,
    },
    Emitter, Manager,
};
use tauri_plugin_opener::OpenerExt;

const DOCUMENTATION_ID: &str = "oomu-documentation";
const SETTINGS_ID: &str = "oomu-settings";
const CHECK_FOR_UPDATES_ID: &str = "oomu-check-for-updates";
const DOCUMENTATION_URL: &str = "https://oomu.ai/docs.html";
const OPEN_SETTINGS_EVENT: &str = "oomu://open-settings";
const CHECK_FOR_UPDATES_EVENT: &str = "oomu://check-for-updates";

struct MenuCopy {
    settings: String,
    help: String,
    documentation: String,
    check_for_updates: String,
}

impl Default for MenuCopy {
    fn default() -> Self {
        Self {
            settings: "Settings…".to_string(),
            help: "Help".to_string(),
            documentation: "OOMU Documentation".to_string(),
            check_for_updates: "Check for Updates…".to_string(),
        }
    }
}

fn localized_copy(translations: Option<&serde_json::Value>) -> MenuCopy {
    let fallback = MenuCopy::default();
    let value = |key: &str, default: &str| {
        translations
            .and_then(|translations| translations.pointer(&format!("/menu_bar/{key}")))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(default)
            .to_string()
    };
    MenuCopy {
        settings: value("settings", &fallback.settings),
        help: value("help", &fallback.help),
        documentation: value("documentation", &fallback.documentation),
        check_for_updates: value("check_for_updates", &fallback.check_for_updates),
    }
}

fn build_with_copy<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &MenuCopy,
    update_ready: bool,
) -> tauri::Result<Menu<R>> {
    let window = window_menu(app)?;
    let help = help_menu(app, copy, update_ready)?;
    Menu::with_items(
        app,
        &[
            #[cfg(target_os = "macos")]
            &application_menu(app, copy)?,
            #[cfg(not(any(
                target_os = "linux",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "netbsd",
                target_os = "openbsd"
            )))]
            &file_menu(app)?,
            &edit_menu(app)?,
            #[cfg(target_os = "macos")]
            &view_menu(app)?,
            &window,
            &help,
        ],
    )
}

fn about_metadata<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> AboutMetadata<'_> {
    let package = app.package_info();
    AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: app.config().bundle.copyright.clone(),
        authors: app
            .config()
            .bundle
            .publisher
            .clone()
            .map(|value| vec![value]),
        ..Default::default()
    }
}

fn window_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )
}

fn help_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &MenuCopy,
    update_ready: bool,
) -> tauri::Result<Submenu<R>> {
    Submenu::with_id_and_items(
        app,
        HELP_SUBMENU_ID,
        &copy.help,
        true,
        &[
            &MenuItem::with_id(
                app,
                DOCUMENTATION_ID,
                &copy.documentation,
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                CHECK_FOR_UPDATES_ID,
                &copy.check_for_updates,
                update_ready,
                None::<&str>,
            )?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::separator(app)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::about(app, None, Some(about_metadata(app)))?,
        ],
    )
}

#[cfg(target_os = "macos")]
fn application_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    copy: &MenuCopy,
) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        app.package_info().name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, None, Some(about_metadata(app)))?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, SETTINGS_ID, &copy.settings, true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}

fn file_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "File",
        true,
        &[
            &PredefinedMenuItem::close_window(app, None)?,
            #[cfg(not(target_os = "macos"))]
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )
}

fn edit_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )
}

#[cfg(target_os = "macos")]
fn view_menu<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Submenu<R>> {
    Submenu::with_items(
        app,
        "View",
        true,
        &[&PredefinedMenuItem::fullscreen(app, None)?],
    )
}

pub(crate) fn build<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<Menu<R>> {
    build_with_copy(app, &MenuCopy::default(), false)
}

pub(crate) fn refresh(
    app: &tauri::AppHandle,
    translations: Option<&serde_json::Value>,
) -> tauri::Result<()> {
    let update_ready = app
        .try_state::<ApplicationUpdateService>()
        .is_some_and(|service| service.ui_ready());
    app.set_menu(build_with_copy(
        app,
        &localized_copy(translations),
        update_ready,
    )?)?;
    Ok(())
}

pub(crate) fn handle_event(app: &tauri::AppHandle, event: MenuEvent) {
    if event.id() == SETTINGS_ID {
        if let Err(error) = app.emit(OPEN_SETTINGS_EVENT, ()) {
            eprintln!("OOMU_MENU_OPEN_SETTINGS_FAILED error={error}");
        }
    } else if event.id() == DOCUMENTATION_ID {
        if let Err(error) = app.opener().open_url(DOCUMENTATION_URL, None::<&str>) {
            eprintln!("OOMU_MENU_OPEN_DOCUMENTATION_FAILED error={error}");
        }
    } else if event.id() == CHECK_FOR_UPDATES_ID {
        background_runtime_tray::restore_foreground(app);
        if let Err(error) = app.emit(CHECK_FOR_UPDATES_EVENT, ()) {
            eprintln!("OOMU_MENU_CHECK_FOR_UPDATES_FAILED error={error}");
        }
    }
}
