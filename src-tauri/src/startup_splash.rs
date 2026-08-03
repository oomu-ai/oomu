//! Minimal native startup presentation for the model-prewarming launch path.
//!
//! The splash intentionally does not own startup policy. Its caller reports a
//! small set of real milestones and decides when the main window is ready. On
//! macOS this is an AppKit window rather than a WebView, so it can be drawn
//! before Tauri constructs the hidden main window.

use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::Duration;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

static MAIN_WINDOW_REVEAL_ALLOWED: AtomicBool = AtomicBool::new(true);
static LAUNCH_STARTED_AT: OnceLock<Instant> = OnceLock::new();
const REVEAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_REVEAL_ATTEMPTS: u8 = 20;

/// Starts the monotonic launch clock before any user-interface initialization.
/// Repeated calls preserve the earliest observed process boundary.
pub(crate) fn begin_launch_timing() {
    if LAUNCH_STARTED_AT.set(Instant::now()).is_ok() {
        emit_startup_milestone("process_started");
    }
}

fn emit_startup_milestone(milestone: &'static str) {
    let started = LAUNCH_STARTED_AT.get_or_init(Instant::now);
    let elapsed_ms = started.elapsed().as_millis();
    let at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    eprintln!(
        "OOMU_STARTUP_MILESTONE milestone={milestone} elapsed_ms={elapsed_ms} at_unix_ms={at_unix_ms}"
    );
}

const LOCALE_CATALOGS: &[(&str, &str)] = &[
    ("de-DE", include_str!("../../src/locales/de-DE.json")),
    ("en-US", include_str!("../../src/locales/en-US.json")),
    ("es-ES", include_str!("../../src/locales/es-ES.json")),
    ("fr-FR", include_str!("../../src/locales/fr-FR.json")),
    ("id-ID", include_str!("../../src/locales/id-ID.json")),
    ("ja-JP", include_str!("../../src/locales/ja-JP.json")),
    ("pt-BR", include_str!("../../src/locales/pt-BR.json")),
    ("ru-RU", include_str!("../../src/locales/ru-RU.json")),
    ("uk-UA", include_str!("../../src/locales/uk-UA.json")),
    ("vi-VN", include_str!("../../src/locales/vi-VN.json")),
    ("zh-CN", include_str!("../../src/locales/zh-CN.json")),
    ("zh-TW", include_str!("../../src/locales/zh-TW.json")),
];

/// Truthful, intentionally coarse launch states. There is no simulated
/// percentage: each state is reported only after the caller reaches that
/// startup boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum StartupMilestone {
    PreparingApplication,
    LoadingStartupModel,
    FinishingStartup,
    Recovery,
    Ready,
}

impl StartupMilestone {
    fn catalog_path(self) -> &'static [&'static str] {
        match self {
            Self::PreparingApplication => &["startup", "splash_preparing"],
            Self::LoadingStartupModel => &["startup", "splash_model"],
            Self::FinishingStartup => &["startup", "splash_finishing"],
            Self::Recovery => &["startup", "splash_recovery"],
            Self::Ready => &["projects", "source_state_ready"],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SplashLifecycle {
    Visible(StartupMilestone),
    Dismissed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RevealDisposition {
    ShellFallback,
    Ready,
    Recovery,
}

#[derive(Debug)]
struct StartupPresentationState {
    lifecycle: SplashLifecycle,
    main_shell_ready: bool,
    requested_reveal: Option<RevealDisposition>,
    reveal_attempts: u8,
    reveal_retry_scheduled: bool,
}

impl StartupPresentationState {
    fn new(use_shell_fallback: bool) -> Self {
        Self {
            lifecycle: SplashLifecycle::Visible(StartupMilestone::PreparingApplication),
            main_shell_ready: false,
            requested_reveal: use_shell_fallback.then_some(RevealDisposition::ShellFallback),
            reveal_attempts: 0,
            reveal_retry_scheduled: false,
        }
    }

    fn requested_reveal_if_ready(&self) -> Option<RevealDisposition> {
        if !self.main_shell_ready || matches!(self.lifecycle, SplashLifecycle::Dismissed) {
            return None;
        }
        self.requested_reveal
    }

    fn mark_main_shell_ready(&mut self) {
        if !self.main_shell_ready {
            self.main_shell_ready = true;
            emit_startup_milestone("main_shell_finished");
        }
    }

    fn request_reveal(&mut self, disposition: RevealDisposition) {
        if matches!(self.lifecycle, SplashLifecycle::Dismissed) {
            return;
        }
        self.requested_reveal = match (self.requested_reveal, disposition) {
            // Recovery is the safe terminal outcome if competing signals ever
            // arrive before presentation completes.
            (Some(RevealDisposition::Recovery), _) | (_, RevealDisposition::Recovery) => {
                Some(RevealDisposition::Recovery)
            }
            (Some(RevealDisposition::ShellFallback), RevealDisposition::Ready) => {
                Some(RevealDisposition::ShellFallback)
            }
            (Some(existing), _) => Some(existing),
            (None, requested) => Some(requested),
        };
    }

    fn record_reveal_failure(&mut self) -> RevealAttempt {
        self.reveal_attempts = self.reveal_attempts.saturating_add(1);
        if self.reveal_attempts < MAX_REVEAL_ATTEMPTS {
            RevealAttempt::Retry
        } else {
            RevealAttempt::Exhausted
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RevealAttempt {
    Waiting,
    Complete,
    Retry,
    Exhausted,
}

impl SplashLifecycle {
    fn report(&mut self, milestone: StartupMilestone) -> bool {
        let Self::Visible(current) = self else {
            return false;
        };
        if milestone <= *current {
            return false;
        }
        *current = milestone;
        true
    }

    fn dismiss(&mut self) -> bool {
        if matches!(self, Self::Dismissed) {
            return false;
        }
        *self = Self::Dismissed;
        true
    }
}

/// Send-safe handle for the main-thread startup presentation.
///
/// Presentation failure is deliberately non-fatal: OOMU must continue its
/// verified startup path even when AppKit cannot create this cosmetic window.
#[derive(Clone)]
pub(crate) struct StartupSplash {
    state: Arc<Mutex<StartupPresentationState>>,
}

impl StartupSplash {
    /// Presents one splash at most. Call this only after the single-instance
    /// lease is acquired, and before opening databases or loading the model.
    pub(crate) fn present() -> Self {
        begin_launch_timing();
        MAIN_WINDOW_REVEAL_ALLOWED.store(false, Ordering::Release);
        let milestone = StartupMilestone::PreparingApplication;
        let status = localized_milestone(milestone);
        #[cfg(target_os = "macos")]
        let splash_available = match macos::present(&status) {
            Ok(()) => {
                emit_startup_milestone("splash_presented");
                true
            }
            Err(code) => {
                eprintln!("OOMU_STARTUP_SPLASH_UNAVAILABLE code={code}");
                false
            }
        };
        #[cfg(not(target_os = "macos"))]
        let splash_available = false;
        Self {
            state: Arc::new(Mutex::new(StartupPresentationState::new(!splash_available))),
        }
    }

    /// Updates the single status line when the caller reaches a real launch
    /// boundary. It is safe to call this from a startup worker: AppKit work is
    /// marshalled onto Tauri's main thread.
    pub(crate) fn report(
        &self,
        app: &tauri::AppHandle,
        milestone: StartupMilestone,
    ) -> tauri::Result<()> {
        let state = Arc::clone(&self.state);
        let status = localized_milestone(milestone);
        app.run_on_main_thread(move || {
            let Ok(mut state) = state.lock() else {
                eprintln!("OOMU_STARTUP_SPLASH_STATE_UNAVAILABLE boundary=report");
                return;
            };
            if !state.lifecycle.report(milestone) {
                return;
            }
            match milestone {
                StartupMilestone::LoadingStartupModel => {
                    emit_startup_milestone("model_preparation_started")
                }
                StartupMilestone::FinishingStartup => emit_startup_milestone("finishing_startup"),
                _ => {}
            }
            #[cfg(target_os = "macos")]
            macos::set_status(&status);
        })
    }

    /// Reveals the fully renderable main window and then removes the splash in
    /// one main-thread transition. A failed main-window reveal deliberately
    /// leaves the splash visible instead of exposing a blank desktop.
    pub(crate) fn reveal_main_when_ready(&self, app: &tauri::AppHandle) -> tauri::Result<()> {
        let state = Arc::clone(&self.state);
        let callback_app = app.clone();
        app.run_on_main_thread(move || {
            if let Ok(mut presentation) = state.lock() {
                presentation.request_reveal(RevealDisposition::Ready);
            } else {
                eprintln!("OOMU_STARTUP_SPLASH_STATE_UNAVAILABLE boundary=ready");
            }
            attempt_reveal(&callback_app, &state, false);
        })
    }

    /// Records that the hidden main WebView has finished loading its shell.
    /// The main window appears only after this and native startup readiness
    /// have both been observed, regardless of which finishes first.
    pub(crate) fn mark_main_shell_ready(&self, app: &tauri::AppHandle) -> tauri::Result<()> {
        let state = Arc::clone(&self.state);
        let callback_app = app.clone();
        app.run_on_main_thread(move || {
            if let Ok(mut presentation) = state.lock() {
                presentation.mark_main_shell_ready();
            } else {
                eprintln!("OOMU_STARTUP_SPLASH_STATE_UNAVAILABLE boundary=main_shell");
            }
            attempt_reveal(&callback_app, &state, false);
        })
    }

    /// Hands presentation to OOMU's recovery surface when startup completes in
    /// a degraded state. This deliberately does not report `Ready`.
    pub(crate) fn reveal_main_for_recovery(&self, app: &tauri::AppHandle) -> tauri::Result<()> {
        let state = Arc::clone(&self.state);
        let callback_app = app.clone();
        let recovery_status = localized_milestone(StartupMilestone::Recovery);
        app.run_on_main_thread(move || {
            #[cfg(target_os = "macos")]
            macos::set_status(&recovery_status);
            if let Ok(mut presentation) = state.lock() {
                presentation.request_reveal(RevealDisposition::Recovery);
            } else {
                eprintln!("OOMU_STARTUP_SPLASH_STATE_UNAVAILABLE boundary=recovery");
            }
            attempt_reveal(&callback_app, &state, false);
        })
    }
}

/// Guards Dock reopen and single-instance activation while the verified main
/// window is intentionally hidden behind startup presentation.
pub(crate) fn main_window_reveal_allowed() -> bool {
    MAIN_WINDOW_REVEAL_ALLOWED.load(Ordering::Acquire)
}

/// Brings the active splash forward when a launch-time activation arrives.
pub(crate) fn refocus_active() {
    #[cfg(target_os = "macos")]
    macos::refocus();
}

fn reveal_if_possible(
    app: &tauri::AppHandle,
    state: &mut StartupPresentationState,
) -> RevealAttempt {
    use tauri::Manager;

    let Some(disposition) = state.requested_reveal_if_ready() else {
        return RevealAttempt::Waiting;
    };
    let Some(main_window) = app.get_webview_window("main") else {
        eprintln!("OOMU_STARTUP_SPLASH_MAIN_REVEAL_FAILED code=main_window_unavailable");
        return state.record_reveal_failure();
    };
    if let Err(error) = main_window.show() {
        eprintln!("OOMU_STARTUP_SPLASH_MAIN_REVEAL_FAILED code=show_failed error={error}");
        return state.record_reveal_failure();
    }
    MAIN_WINDOW_REVEAL_ALLOWED.store(true, Ordering::Release);
    if let Err(error) = main_window.set_focus() {
        eprintln!("OOMU_STARTUP_SPLASH_MAIN_FOCUS_FAILED error={error}");
    }
    emit_startup_milestone(match disposition {
        RevealDisposition::ShellFallback => "main_shown_shell_fallback",
        RevealDisposition::Ready => "main_shown_ready",
        RevealDisposition::Recovery => "main_shown_recovery",
    });
    match disposition {
        RevealDisposition::ShellFallback => {}
        RevealDisposition::Ready => {
            state.lifecycle.report(StartupMilestone::Ready);
        }
        RevealDisposition::Recovery => {
            state.lifecycle.report(StartupMilestone::Recovery);
        }
    }
    state.requested_reveal = None;
    if state.lifecycle.dismiss() {
        #[cfg(target_os = "macos")]
        macos::close();
    }
    RevealAttempt::Complete
}

fn attempt_reveal(
    app: &tauri::AppHandle,
    presentation: &Arc<Mutex<StartupPresentationState>>,
    scheduled_retry: bool,
) {
    let outcome = {
        let Ok(mut state) = presentation.lock() else {
            eprintln!("OOMU_STARTUP_SPLASH_STATE_UNAVAILABLE boundary=reveal");
            return;
        };
        if state.reveal_retry_scheduled && !scheduled_retry {
            return;
        }
        if scheduled_retry {
            state.reveal_retry_scheduled = false;
        }
        reveal_if_possible(app, &mut state)
    };
    match outcome {
        RevealAttempt::Retry => schedule_reveal_retry(app.clone(), Arc::clone(presentation)),
        RevealAttempt::Exhausted => fail_startup_presentation(app, presentation),
        RevealAttempt::Waiting | RevealAttempt::Complete => {}
    }
}

fn schedule_reveal_retry(
    app: tauri::AppHandle,
    presentation: Arc<Mutex<StartupPresentationState>>,
) {
    let should_schedule = presentation.lock().is_ok_and(|mut state| {
        if state.reveal_retry_scheduled || matches!(state.lifecycle, SplashLifecycle::Dismissed) {
            return false;
        }
        state.reveal_retry_scheduled = true;
        true
    });
    if !should_schedule {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REVEAL_RETRY_DELAY).await;
        let callback_app = app.clone();
        let retry_state = Arc::clone(&presentation);
        if let Err(error) = app.run_on_main_thread(move || {
            attempt_reveal(&callback_app, &retry_state, true);
        }) {
            eprintln!("OOMU_STARTUP_SPLASH_RETRY_DISPATCH_FAILED error={error}");
            app.exit(1);
        }
    });
}

fn fail_startup_presentation(
    app: &tauri::AppHandle,
    presentation: &Arc<Mutex<StartupPresentationState>>,
) {
    if let Ok(mut state) = presentation.lock() {
        state.lifecycle.dismiss();
    }
    #[cfg(target_os = "macos")]
    macos::close();
    emit_startup_milestone("main_reveal_failed");
    let error = crate::errors::OomuError::StartupIntegrity {
        code: "startup_window_unavailable",
        detail: "the main application window could not be presented after bounded retries"
            .to_string(),
    };
    crate::startup_integrity_ui::show(&error);
    app.exit(1);
}

/// Last-resort main-thread cleanup for failures returned before a handle can
/// be threaded through Tauri's setup lifecycle.
pub(crate) fn dismiss_active_for_failure() {
    #[cfg(target_os = "macos")]
    macos::close();
}

fn localized_milestone(milestone: StartupMilestone) -> String {
    localized_text(&selected_catalog(), milestone.catalog_path())
        .expect("every embedded locale must provide startup splash copy")
        .to_string()
}

fn selected_catalog() -> Value {
    catalog_for_locale(&preferred_locale())
}

fn catalog_for_locale(preferred: &str) -> Value {
    let language = preferred.split(['-', '_']).next().unwrap_or_default();
    let (_, source) = LOCALE_CATALOGS
        .iter()
        .find(|(locale, _)| locale.eq_ignore_ascii_case(preferred))
        .or_else(|| {
            LOCALE_CATALOGS
                .iter()
                .find(|(locale, _)| locale.starts_with(language))
        })
        .or_else(|| {
            LOCALE_CATALOGS
                .iter()
                .find(|(locale, _)| *locale == "en-US")
        })
        .expect("the embedded US English locale must exist");
    serde_json::from_str(source).expect("embedded startup locale JSON must be valid")
}

fn localized_text<'a>(catalog: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut value = catalog;
    for key in path {
        value = value.get(key)?;
    }
    value.as_str().filter(|text| !text.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn preferred_locale() -> String {
    use objc2_foundation::NSLocale;

    NSLocale::preferredLanguages()
        .firstObject()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "en-US".to_string())
}

#[cfg(not(target_os = "macos"))]
fn preferred_locale() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|value| value.split('.').next().map(str::to_string))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "en-US".to_string())
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::{
        msg_send,
        rc::autoreleasepool,
        runtime::{AnyClass, AnyObject},
        MainThreadMarker,
    };
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::{
        ptr::NonNull,
        sync::{Mutex, OnceLock},
    };

    const SPLASH_WIDTH: f64 = 440.0;
    const SPLASH_HEIGHT: f64 = 270.0;
    const RAVEN_SVG: &[u8] = include_bytes!("../../public/oomu-raven.svg");

    static ACTIVE_SPLASH: OnceLock<Mutex<Option<NativeSplash>>> = OnceLock::new();

    struct NativeSplash {
        // Addresses are stored as integers solely so the global controller can
        // be captured by Send Tauri startup closures. Every dereference is
        // guarded by MainThreadMarker in this module.
        window: usize,
        status_label: usize,
        closed: bool,
    }

    impl NativeSplash {
        fn set_status(&mut self, status: &str) {
            if self.closed {
                return;
            }
            autoreleasepool(|_| unsafe {
                let status = NSString::from_str(status);
                let status_label = self.status_label as *mut AnyObject;
                let window = self.window as *mut AnyObject;
                let _: () = msg_send![status_label, setStringValue: &*status];
                force_display(window);
            });
        }

        fn close(&mut self) {
            if self.closed {
                return;
            }
            self.closed = true;
            autoreleasepool(|_| unsafe {
                let window = self.window as *mut AnyObject;
                let status_label = self.status_label as *mut AnyObject;
                let _: () = msg_send![status_label, release];
                let _: () = msg_send![window, orderOut: std::ptr::null::<AnyObject>()];
                let _: () = msg_send![window, close];
                let _: () = msg_send![window, release];
            });
        }
    }

    pub(super) fn present(status: &str) -> Result<(), &'static str> {
        let mut active = active_splash().lock().map_err(|_| "state_poisoned")?;
        if active.is_some() {
            return Err("already_presented");
        }
        let native = autoreleasepool(|_| unsafe { present_on_main_thread(status) })?;
        *active = Some(native);
        Ok(())
    }

    pub(super) fn set_status(status: &str) {
        if MainThreadMarker::new().is_none() {
            eprintln!("OOMU_STARTUP_SPLASH_UPDATE_SKIPPED code=not_main_thread");
            return;
        }
        if let Ok(mut active) = active_splash().lock() {
            if let Some(native) = active.as_mut() {
                native.set_status(status);
            }
        }
    }

    pub(super) fn close() {
        if MainThreadMarker::new().is_none() {
            eprintln!("OOMU_STARTUP_SPLASH_CLOSE_SKIPPED code=not_main_thread");
            return;
        }
        if let Ok(mut active) = active_splash().lock() {
            if let Some(mut native) = active.take() {
                native.close();
            }
        }
    }

    pub(super) fn refocus() {
        if MainThreadMarker::new().is_none() {
            eprintln!("OOMU_STARTUP_SPLASH_REFOCUS_SKIPPED code=not_main_thread");
            return;
        }
        if let Ok(active) = active_splash().lock() {
            if let Some(native) = active.as_ref() {
                unsafe {
                    let window = native.window as *mut AnyObject;
                    let _: () = msg_send![window, orderFrontRegardless];
                    force_display(window);
                }
            }
        }
    }

    fn active_splash() -> &'static Mutex<Option<NativeSplash>> {
        ACTIVE_SPLASH.get_or_init(|| Mutex::new(None))
    }

    unsafe fn present_on_main_thread(status: &str) -> Result<NativeSplash, &'static str> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Err("not_main_thread");
        };
        let _application = NSApplication::sharedApplication(mtm);
        let window_class = required_class("NSWindow")?;
        let effect_class = required_class("NSVisualEffectView")?;
        let image_view_class = required_class("NSImageView")?;
        let text_field_class = required_class("NSTextField")?;
        let font_class = required_class("NSFont")?;
        let color_class = required_class("NSColor")?;
        let data_class = required_class("NSData")?;
        let image_class = required_class("NSImage")?;

        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(SPLASH_WIDTH, SPLASH_HEIGHT),
        );
        let allocated_window: *mut AnyObject = msg_send![window_class, alloc];
        let window: *mut AnyObject = msg_send![allocated_window,
            initWithContentRect: frame,
            styleMask: 0_usize,
            backing: 2_usize,
            defer: false
        ];
        let Some(window) = NonNull::new(window) else {
            return Err("window_creation_failed");
        };

        let root = allocate_view(effect_class, frame).ok_or("content_creation_failed")?;
        let clear_color: *mut AnyObject = msg_send![color_class, clearColor];
        let _: () = msg_send![window.as_ptr(), setOpaque: false];
        let _: () = msg_send![window.as_ptr(), setBackgroundColor: clear_color];
        let _: () = msg_send![window.as_ptr(), setHasShadow: true];
        let _: () = msg_send![window.as_ptr(), setReleasedWhenClosed: false];
        let _: () = msg_send![root, setMaterial: 12_isize];
        let _: () = msg_send![root, setBlendingMode: 0_isize];
        let _: () = msg_send![root, setState: 1_isize];
        let _: () = msg_send![root, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![root, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setCornerRadius: 24.0_f64];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }
        let _: () = msg_send![window.as_ptr(), setContentView: root];
        let _: () = msg_send![root, release];

        let image_frame = NSRect::new(NSPoint::new(176.0, 137.0), NSSize::new(88.0, 79.0));
        let image_view =
            allocate_view(image_view_class, image_frame).ok_or("image_view_creation_failed")?;
        let data: *mut AnyObject = msg_send![data_class,
            dataWithBytes: RAVEN_SVG.as_ptr(),
            length: RAVEN_SVG.len()
        ];
        let allocated_image: *mut AnyObject = msg_send![image_class, alloc];
        let image: *mut AnyObject = msg_send![allocated_image, initWithData: data];
        if !image.is_null() {
            let label_color: *mut AnyObject = msg_send![color_class, labelColor];
            let _: () = msg_send![image, setTemplate: true];
            let _: () = msg_send![image_view, setImage: image];
            let _: () = msg_send![image_view, setImageScaling: 3_usize];
            let _: () = msg_send![image_view, setContentTintColor: label_color];
            let _: () = msg_send![image, release];
        }
        let _: () = msg_send![root, addSubview: image_view];
        let _: () = msg_send![image_view, release];

        let title_frame = NSRect::new(NSPoint::new(40.0, 91.0), NSSize::new(360.0, 40.0));
        let title = text_label(
            text_field_class,
            font_class,
            color_class,
            title_frame,
            "OOMU",
            30.0,
            0.38,
            false,
        )
        .ok_or("title_creation_failed")?;
        let _: () = msg_send![root, addSubview: title];
        let _: () = msg_send![title, release];

        let status_frame = NSRect::new(NSPoint::new(40.0, 57.0), NSSize::new(360.0, 24.0));
        let status_label = text_label(
            text_field_class,
            font_class,
            color_class,
            status_frame,
            status,
            13.0,
            0.0,
            true,
        )
        .ok_or("status_creation_failed")?;
        let _: () = msg_send![root, addSubview: status_label];
        // Keep our +1 reference because milestone updates address this label
        // directly. The view hierarchy also retains it until window teardown.

        let _: () = msg_send![window.as_ptr(), center];
        let _: () = msg_send![window.as_ptr(), orderFrontRegardless];
        force_display(window.as_ptr());

        Ok(NativeSplash {
            window: window.as_ptr() as usize,
            status_label: NonNull::new(status_label)
                .expect("label was checked above")
                .as_ptr() as usize,
            closed: false,
        })
    }

    unsafe fn required_class(name: &str) -> Result<&'static AnyClass, &'static str> {
        match name {
            "NSWindow" => AnyClass::get(c"NSWindow").ok_or("window_class_unavailable"),
            "NSVisualEffectView" => {
                AnyClass::get(c"NSVisualEffectView").ok_or("effect_class_unavailable")
            }
            "NSImageView" => AnyClass::get(c"NSImageView").ok_or("image_view_class_unavailable"),
            "NSTextField" => AnyClass::get(c"NSTextField").ok_or("text_class_unavailable"),
            "NSFont" => AnyClass::get(c"NSFont").ok_or("font_class_unavailable"),
            "NSColor" => AnyClass::get(c"NSColor").ok_or("color_class_unavailable"),
            "NSData" => AnyClass::get(c"NSData").ok_or("data_class_unavailable"),
            "NSImage" => AnyClass::get(c"NSImage").ok_or("image_class_unavailable"),
            _ => Err("unknown_class"),
        }
    }

    unsafe fn allocate_view(class: &AnyClass, frame: NSRect) -> Option<*mut AnyObject> {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        NonNull::new(msg_send![allocated, initWithFrame: frame]).map(NonNull::as_ptr)
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn text_label(
        text_field_class: &AnyClass,
        font_class: &AnyClass,
        color_class: &AnyClass,
        frame: NSRect,
        value: &str,
        size: f64,
        weight: f64,
        secondary: bool,
    ) -> Option<*mut AnyObject> {
        let label = allocate_view(text_field_class, frame)?;
        let value = NSString::from_str(value);
        let font: *mut AnyObject = msg_send![font_class, systemFontOfSize: size, weight: weight];
        let color: *mut AnyObject = if secondary {
            msg_send![color_class, secondaryLabelColor]
        } else {
            msg_send![color_class, labelColor]
        };
        let _: () = msg_send![label, setStringValue: &*value];
        let _: () = msg_send![label, setFont: font];
        let _: () = msg_send![label, setTextColor: color];
        let _: () = msg_send![label, setAlignment: 1_isize];
        let _: () = msg_send![label, setBezeled: false];
        let _: () = msg_send![label, setDrawsBackground: false];
        let _: () = msg_send![label, setEditable: false];
        let _: () = msg_send![label, setSelectable: false];
        Some(label)
    }

    unsafe fn force_display(window: *mut AnyObject) {
        let content: *mut AnyObject = msg_send![window, contentView];
        if !content.is_null() {
            let _: () = msg_send![content, displayIfNeeded];
        }
        let _: () = msg_send![window, displayIfNeeded];
        let _: () = msg_send![window, flushWindow];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_cannot_move_backwards_or_resume_after_dismissal() {
        let mut lifecycle = SplashLifecycle::Visible(StartupMilestone::PreparingApplication);
        assert!(lifecycle.report(StartupMilestone::LoadingStartupModel));
        assert!(!lifecycle.report(StartupMilestone::PreparingApplication));
        assert!(lifecycle.report(StartupMilestone::FinishingStartup));
        assert!(lifecycle.dismiss());
        assert!(!lifecycle.report(StartupMilestone::Ready));
        assert!(!lifecycle.dismiss());
    }

    #[test]
    fn native_readiness_waits_for_the_main_shell() {
        let mut state = StartupPresentationState::new(false);
        state.request_reveal(RevealDisposition::Ready);
        assert_eq!(state.requested_reveal_if_ready(), None);
        state.mark_main_shell_ready();
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::Ready)
        );
    }

    #[test]
    fn main_shell_readiness_waits_for_the_native_outcome() {
        let mut state = StartupPresentationState::new(false);
        state.mark_main_shell_ready();
        assert_eq!(state.requested_reveal_if_ready(), None);
        state.request_reveal(RevealDisposition::Recovery);
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::Recovery)
        );
    }

    #[test]
    fn duplicate_signals_are_idempotent_and_recovery_wins_before_reveal() {
        let mut state = StartupPresentationState::new(false);
        state.mark_main_shell_ready();
        state.mark_main_shell_ready();
        state.request_reveal(RevealDisposition::Ready);
        state.request_reveal(RevealDisposition::Ready);
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::Ready)
        );
        state.request_reveal(RevealDisposition::Recovery);
        state.request_reveal(RevealDisposition::Ready);
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::Recovery)
        );
        state.lifecycle.dismiss();
        state.request_reveal(RevealDisposition::Ready);
        assert_eq!(state.requested_reveal_if_ready(), None);
    }

    #[test]
    fn failed_window_reveal_has_a_bounded_retry_budget() {
        let mut state = StartupPresentationState::new(false);
        for _ in 1..MAX_REVEAL_ATTEMPTS {
            assert_eq!(state.record_reveal_failure(), RevealAttempt::Retry);
        }
        assert_eq!(state.record_reveal_failure(), RevealAttempt::Exhausted);
    }

    #[test]
    fn unavailable_native_splash_reveals_the_rendered_shell_without_waiting_for_prewarm() {
        let mut state = StartupPresentationState::new(true);
        assert_eq!(state.requested_reveal_if_ready(), None);
        state.mark_main_shell_ready();
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::ShellFallback)
        );
        state.request_reveal(RevealDisposition::Ready);
        assert_eq!(
            state.requested_reveal_if_ready(),
            Some(RevealDisposition::ShellFallback)
        );
    }

    #[test]
    fn every_locale_has_nonempty_copy_for_each_truthful_milestone() {
        for (_, source) in LOCALE_CATALOGS {
            let catalog: Value = serde_json::from_str(source).unwrap();
            for milestone in [
                StartupMilestone::PreparingApplication,
                StartupMilestone::LoadingStartupModel,
                StartupMilestone::FinishingStartup,
                StartupMilestone::Recovery,
                StartupMilestone::Ready,
            ] {
                assert!(localized_text(&catalog, milestone.catalog_path()).is_some());
            }
        }
    }

    #[test]
    fn locale_matching_supports_exact_language_and_english_fallback() {
        assert_eq!(
            localized_text(
                &catalog_for_locale("ja-JP"),
                StartupMilestone::Ready.catalog_path()
            ),
            Some("準備完了")
        );
        assert_eq!(
            localized_text(
                &catalog_for_locale("ja"),
                StartupMilestone::Ready.catalog_path()
            ),
            Some("準備完了")
        );
        assert_eq!(
            localized_text(
                &catalog_for_locale("xx-Unknown"),
                StartupMilestone::Ready.catalog_path()
            ),
            Some("Ready")
        );
    }

    #[test]
    fn configured_main_window_stays_hidden_until_verified_reveal() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            config["app"]["windows"][0]["visible"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn splash_uses_the_canonical_raven_without_a_duplicate_asset() {
        let raven = include_str!("../../public/oomu-raven.svg");
        assert!(raven.contains("<svg"));
        assert!(raven.contains("<path"));
        assert!(!raven.contains("<script"));
    }
}
