mod postconditions;
mod receipts;
mod state;
mod worker;

use super::BackgroundServiceStatus;
use crate::db::PersistenceEngine;
use std::{thread, time::Duration};

pub use worker::BackgroundRuntimeSupervisor;

pub(crate) const BACKGROUND_RUNTIME_STATUS_EVENT: &str = "oomu://background-runtime-status";
const VERIFICATION_WAIT: Duration = Duration::from_secs(4);
const VERIFICATION_POLL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScheduledWorkOwner {
    ForegroundApplication,
    DetachedRuntime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StartupReconciliation {
    requested_enabled: bool,
    registration_generation: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegistrationBackend {
    SmAppService,
    SupervisedProcess,
}

impl RegistrationBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::SmAppService => "sm_app_service",
            Self::SupervisedProcess => "supervised_process",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "sm_app_service" => Some(Self::SmAppService),
            "supervised_process" => Some(Self::SupervisedProcess),
            _ => None,
        }
    }
}

pub fn should_keep_alive(engine: &PersistenceEngine) -> bool {
    status(engine)
        .map(|status| status.verified_active)
        .unwrap_or(false)
}

pub(super) fn set_enabled(
    app: tauri::AppHandle,
    engine: &PersistenceEngine,
    supervisor: &BackgroundRuntimeSupervisor,
    enabled: bool,
) -> Result<BackgroundServiceStatus, String> {
    let _transition_guard = supervisor.lock_transition()?;
    state::begin_transition(engine, enabled, "requested_state_changed")?;
    if enabled {
        set_enabled_inner(app, engine, supervisor)
    } else {
        set_disabled_inner(engine, supervisor)
    }
}

fn set_enabled_inner(
    app: tauri::AppHandle,
    engine: &PersistenceEngine,
    supervisor: &BackgroundRuntimeSupervisor,
) -> Result<BackgroundServiceStatus, String> {
    let backend = match registration_backend() {
        Ok(backend) => backend,
        Err(code) => {
            let code = background_registration_error(&code);
            state::set_attention(engine, code)?;
            return state::status(engine);
        }
    };
    state::set_registration_backend(engine, backend.as_str())?;
    state::registration_started(engine)?;
    let mutation = service_register(backend);
    let (observation, error_code) = observed_registration(engine, backend, mutation, true);
    let row = state::observe_registration(engine, observation, error_code.as_deref())?;
    if observation != state::RegistrationObservation::Registered {
        state::registration_receipt(engine, false, error_code.as_deref())?;
        state::set_attention(
            engine,
            error_code
                .as_deref()
                .unwrap_or("background_registration_not_verified"),
        )?;
        return status(engine);
    }
    state::registration_receipt(engine, true, None)?;
    let generation = row
        .registration_generation
        .as_deref()
        .ok_or_else(|| "background_registration_generation_missing".to_string())?;
    if let Err(code) = supervisor.ensure_started(
        app,
        engine.clone(),
        generation,
        &row.profile_generation,
        row.build_number,
        &row.build_identity,
        &row.profile_class,
    ) {
        state::set_attention(engine, &code)?;
        return status(engine);
    }
    wait_for_verified_status(engine)
}

fn set_disabled_inner(
    engine: &PersistenceEngine,
    supervisor: &BackgroundRuntimeSupervisor,
) -> Result<BackgroundServiceStatus, String> {
    let backend = registration_backend_for_observation(engine)?;
    state::set_registration_backend(engine, backend.as_str())?;
    let generation = state::generation(engine).ok().map(|value| value.0);
    supervisor.stop()?;
    if let Some(generation) = generation.as_deref() {
        let _ = state::record_worker_stopped(engine, generation, true);
    }
    let (observation, error_code) = reconcile_disabled_registration(
        || service_status(engine, backend),
        || service_unregister(backend),
    );
    state::observe_registration(engine, observation, error_code.as_deref())?;
    if observation == state::RegistrationObservation::Unregistered {
        state::finish_disabled(engine)?;
        state::registration_receipt(engine, true, None)?;
    } else {
        state::set_attention(
            engine,
            error_code
                .as_deref()
                .unwrap_or("background_unregistration_not_verified"),
        )?;
        state::registration_receipt(engine, false, error_code.as_deref())?;
    }
    status(engine)
}

pub(crate) fn status(engine: &PersistenceEngine) -> Result<BackgroundServiceStatus, String> {
    state::ensure_state(engine)?;
    let backend = match registration_backend_for_observation(engine) {
        Ok(backend) => backend,
        Err(_code) if !state::requested(engine)? => return state::status(engine),
        Err(code) => {
            state::set_attention(engine, background_registration_error(&code))?;
            return state::status(engine);
        }
    };
    state::set_registration_backend(engine, backend.as_str())?;
    let requested = state::requested(engine)?;
    let (observation, error_code) = match service_status(engine, backend) {
        Ok(observed) => {
            let observation = registration_observation(&observed);
            let error = registration_status_error(requested, observation).map(str::to_string);
            (observation, error)
        }
        Err(code) => (registration_failure_observation(&code), Some(code)),
    };
    state::observe_registration(engine, observation, error_code.as_deref())?;
    state::status(engine)
}

pub(crate) fn menu_should_be_visible(engine: &PersistenceEngine) -> Result<bool, String> {
    state::menu_activation_ready(engine)
}

pub(crate) fn begin_startup_reconciliation(
    engine: &PersistenceEngine,
) -> Result<StartupReconciliation, String> {
    state::ensure_state(engine)?;
    state::record_menu_visibility(engine, false)?;
    let requested_enabled = state::requested(engine)?;
    let row = state::begin_transition(engine, requested_enabled, "reconciliation_started")?;
    Ok(StartupReconciliation {
        requested_enabled,
        registration_generation: row.registration_generation,
    })
}

pub(crate) fn finish_startup_reconciliation(
    app: tauri::AppHandle,
    engine: &PersistenceEngine,
    supervisor: &BackgroundRuntimeSupervisor,
    reconciliation: &StartupReconciliation,
) -> Result<BackgroundServiceStatus, String> {
    let _transition_guard = supervisor.lock_transition()?;
    let current = state::ensure_state(engine)?;
    if current.requested_enabled != reconciliation.requested_enabled
        || current.registration_generation != reconciliation.registration_generation
    {
        return state::status(engine);
    }
    let result = if reconciliation.requested_enabled {
        set_enabled_inner(app, engine, supervisor)
    } else {
        set_disabled_inner(engine, supervisor)
    };
    match result {
        Ok(status) => {
            let verified = matches!(status.state.as_str(), "on_verified" | "off");
            if verified {
                state::record_reconciliation(engine, true, None)?;
            } else if status.state != "turning_on" {
                state::record_reconciliation(
                    engine,
                    false,
                    Some("background_reconciliation_incomplete"),
                )?;
            }
            Ok(status)
        }
        Err(code) => {
            let _ = state::set_attention(engine, &code);
            let _ = state::record_reconciliation(engine, false, Some(&code));
            status(engine)
        }
    }
}

pub(crate) fn prepare_explicit_quit(
    engine: &PersistenceEngine,
    supervisor: &BackgroundRuntimeSupervisor,
) -> Result<(), String> {
    state::record_runtime_event(engine, "quit_requested")?;
    let generation = state::generation(engine).ok().map(|value| value.0);
    supervisor.stop().map_err(|code| {
        let _ = state::set_attention(engine, &code);
        code
    })?;
    if let Some(generation) = generation.as_deref() {
        state::record_worker_stopped(engine, generation, true)?;
    }
    Ok(())
}

pub(crate) fn record_window_closed(engine: &PersistenceEngine) {
    let _ = state::record_runtime_event(engine, "window_closed");
}

pub(crate) fn record_window_reopened(engine: &PersistenceEngine) {
    let _ = state::record_runtime_event(engine, "window_reopened");
}

pub(crate) fn record_menu_visibility(
    engine: &PersistenceEngine,
    visible: bool,
) -> Result<(), String> {
    state::record_menu_visibility(engine, visible)
}

pub(crate) fn record_runtime_attention(engine: &PersistenceEngine, code: &str) {
    let _ = state::set_attention(engine, code);
}

pub(crate) fn record_disabled_verified(engine: &PersistenceEngine) {
    let _ = state::record_runtime_event(engine, "shutdown_verified");
}

pub(crate) fn record_verified_schedule_completion(
    engine: &PersistenceEngine,
    schedule_id: &str,
    task_run_id: &str,
) -> Result<bool, String> {
    postconditions::record_verified_schedule_completion(engine, schedule_id, task_run_id)
}

pub(crate) fn scheduled_file_postcondition_required(
    engine: &PersistenceEngine,
    task_run_id: &str,
) -> Result<bool, String> {
    postconditions::scheduled_file_postcondition_required(engine, task_run_id)
}

pub(crate) fn scheduled_work_allowed(
    engine: &PersistenceEngine,
    owner: ScheduledWorkOwner,
) -> bool {
    match owner {
        ScheduledWorkOwner::ForegroundApplication => true,
        ScheduledWorkOwner::DetachedRuntime => match state::requested(engine) {
            Ok(false) => true,
            Ok(true) => should_keep_alive(engine),
            Err(_) => false,
        },
    }
}

pub(crate) fn run_worker_if_requested() -> bool {
    worker::run_worker_if_requested()
}

fn wait_for_verified_status(engine: &PersistenceEngine) -> Result<BackgroundServiceStatus, String> {
    let started = std::time::Instant::now();
    loop {
        let status = status(engine)?;
        if menu_should_be_visible(engine)?
            || status.state != "turning_on"
            || started.elapsed() >= VERIFICATION_WAIT
        {
            return Ok(status);
        }
        thread::sleep(VERIFICATION_POLL);
    }
}

fn observed_registration(
    engine: &PersistenceEngine,
    backend: RegistrationBackend,
    mutation: Result<String, String>,
    enabling: bool,
) -> (state::RegistrationObservation, Option<String>) {
    observed_registration_with(mutation, enabling, || service_status(engine, backend))
}

fn observed_registration_with(
    mutation: Result<String, String>,
    enabling: bool,
    mut observe: impl FnMut() -> Result<String, String>,
) -> (state::RegistrationObservation, Option<String>) {
    match mutation {
        Ok(_) => match observe() {
            Ok(observed) => {
                let observation = registration_observation(&observed);
                let matches = if enabling {
                    observation == state::RegistrationObservation::Registered
                } else {
                    observation == state::RegistrationObservation::Unregistered
                };
                (
                    observation,
                    (!matches).then(|| {
                        if enabling {
                            "background_registration_not_active"
                        } else {
                            "background_unregistration_not_paused"
                        }
                        .to_string()
                    }),
                )
            }
            Err(code) => (registration_failure_observation(&code), Some(code)),
        },
        Err(code) => (registration_failure_observation(&code), Some(code)),
    }
}

fn reconcile_disabled_registration(
    mut observe: impl FnMut() -> Result<String, String>,
    unregister: impl FnOnce() -> Result<String, String>,
) -> (state::RegistrationObservation, Option<String>) {
    if let Ok(observed) = observe() {
        let observation = registration_observation(&observed);
        if observation == state::RegistrationObservation::Unregistered {
            return (observation, None);
        }
    }
    observed_registration_with(unregister(), false, observe)
}

fn registration_backend() -> Result<RegistrationBackend, String> {
    let identity = crate::macos_process_identity::current();
    let class = crate::runtime_profile::current_class(&identity)
        .map_err(|failure| failure.code.to_string())?;
    Ok(registration_backend_for(
        class,
        executable_is_in_app_bundle(),
    ))
}

fn registration_backend_for_observation(
    engine: &PersistenceEngine,
) -> Result<RegistrationBackend, String> {
    if let Some(backend) = RegistrationBackend::from_str(&state::registration_backend(engine)?) {
        return Ok(backend);
    }
    if executable_is_in_app_bundle() {
        return Ok(RegistrationBackend::SmAppService);
    }
    registration_backend()
}

fn background_registration_error(code: &str) -> &str {
    if code == crate::runtime_profile::INVALID_PRODUCTION_IDENTITY {
        "background_requires_signed_install"
    } else {
        code
    }
}

fn registration_backend_for(
    class: crate::runtime_profile::RuntimeProfileClass,
    app_bundle: bool,
) -> RegistrationBackend {
    if app_bundle || class == crate::runtime_profile::RuntimeProfileClass::Production {
        RegistrationBackend::SmAppService
    } else {
        RegistrationBackend::SupervisedProcess
    }
}

fn executable_is_in_app_bundle() -> bool {
    std::env::current_exe().ok().is_some_and(|path| {
        path.components()
            .map(|component| component.as_os_str())
            .collect::<Vec<_>>()
            .windows(3)
            .any(|window| {
                window[0].to_string_lossy().ends_with(".app")
                    && window[1] == "Contents"
                    && window[2] == "MacOS"
            })
    })
}

fn registration_observation(observed: &str) -> state::RegistrationObservation {
    match observed {
        "active" => state::RegistrationObservation::Registered,
        "paused" => state::RegistrationObservation::Unregistered,
        "requires_approval" => state::RegistrationObservation::RequiresApproval,
        "unavailable" => state::RegistrationObservation::Unavailable,
        _ => state::RegistrationObservation::Failed,
    }
}

fn registration_failure_observation(code: &str) -> state::RegistrationObservation {
    match code {
        "background_requires_approval" => state::RegistrationObservation::RequiresApproval,
        "background_requires_signed_install" | "smappservice_unavailable" => {
            state::RegistrationObservation::Unavailable
        }
        _ => state::RegistrationObservation::Failed,
    }
}

fn registration_status_error(
    requested: bool,
    observation: state::RegistrationObservation,
) -> Option<&'static str> {
    match (requested, observation) {
        (true, state::RegistrationObservation::Registered)
        | (false, state::RegistrationObservation::Unregistered) => None,
        (true, state::RegistrationObservation::RequiresApproval) => {
            Some("background_requires_approval")
        }
        (true, _) => Some("background_registration_lost"),
        (false, _) => Some("background_unregistration_not_paused"),
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        ffi::{c_char, c_void, CStr},
        ptr,
    };
    pub(super) const MAIN_APP_SERVICE_SELECTOR: &CStr = c"mainAppService";
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    unsafe fn responds_to_selector(receiver: *mut c_void, selector: *mut c_void) -> bool {
        let responds = unsafe { sel_registerName(c"respondsToSelector:".as_ptr()) };
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i8 =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(receiver, responds, selector) != 0 }
    }

    unsafe fn service() -> Result<*mut c_void, String> {
        let class = unsafe { objc_getClass(c"SMAppService".as_ptr()) };
        if class.is_null() {
            return Err("smappservice_unavailable".to_string());
        }
        // `mainApp` is the Swift spelling. Raw Objective-C messaging must use
        // the SDK-declared getter `mainAppService`; the Swift-only selector
        // raises an Objective-C exception that Rust cannot unwind safely.
        let selector = unsafe { sel_registerName(MAIN_APP_SERVICE_SELECTOR.as_ptr()) };
        if !unsafe { responds_to_selector(class, selector) } {
            return Err("smappservice_main_app_unavailable".to_string());
        }
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        let value = unsafe { send(class, selector) };
        if value.is_null() {
            Err("smappservice_main_app_unavailable".to_string())
        } else {
            Ok(value)
        }
    }
    pub fn mutate(register: bool) -> Result<String, String> {
        unsafe {
            let service = service()?;
            let name = if register {
                c"registerAndReturnError:"
            } else {
                c"unregisterAndReturnError:"
            };
            let selector = sel_registerName(name.as_ptr());
            if !responds_to_selector(service, selector) {
                return Err("background_service_api_unavailable".to_string());
            }
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i8 =
                std::mem::transmute(objc_msgSend as *const ());
            let mut error = ptr::null_mut();
            if send(service, selector, &mut error) == 0 {
                let code = error_code(error);
                eprintln!(
                    "OOMU_BACKGROUND_SERVICE_MUTATION_REJECTED register={register} code={code}"
                );
                if (register && code == 12) || (!register && code == 6) {
                    return Ok(if register { "active" } else { "paused" }.to_string());
                }
                return Err(match code {
                    3 => "background_requires_signed_install",
                    11 => "background_requires_approval",
                    _ if register => "background_registration_rejected",
                    _ => "background_unregistration_rejected",
                }
                .to_string());
            }
            Ok(if register { "active" } else { "paused" }.to_string())
        }
    }

    unsafe fn error_code(error: *mut c_void) -> isize {
        if error.is_null() {
            return 0;
        }
        let selector = unsafe { sel_registerName(c"code".as_ptr()) };
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
            unsafe { std::mem::transmute(objc_msgSend as *const ()) };
        unsafe { send(error, selector) }
    }
    pub fn status() -> Result<String, String> {
        unsafe {
            let service = service()?;
            let selector = sel_registerName(c"status".as_ptr());
            if !responds_to_selector(service, selector) {
                return Err("background_service_api_unavailable".to_string());
            }
            let send: unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize =
                std::mem::transmute(objc_msgSend as *const ());
            Ok(match send(service, selector) {
                0 => "paused",
                1 => "active",
                2 => "requires_approval",
                _ => "unavailable",
            }
            .to_string())
        }
    }
}

fn service_register(backend: RegistrationBackend) -> Result<String, String> {
    match backend {
        #[cfg(target_os = "macos")]
        RegistrationBackend::SmAppService => macos::mutate(true),
        #[cfg(not(target_os = "macos"))]
        RegistrationBackend::SmAppService => Err("background_service_requires_macos".to_string()),
        RegistrationBackend::SupervisedProcess => Ok("active".to_string()),
    }
}
fn service_unregister(backend: RegistrationBackend) -> Result<String, String> {
    match backend {
        #[cfg(target_os = "macos")]
        RegistrationBackend::SmAppService => macos::mutate(false),
        #[cfg(not(target_os = "macos"))]
        RegistrationBackend::SmAppService => Err("background_service_requires_macos".to_string()),
        RegistrationBackend::SupervisedProcess => Ok("paused".to_string()),
    }
}
fn service_status(
    engine: &PersistenceEngine,
    backend: RegistrationBackend,
) -> Result<String, String> {
    match backend {
        #[cfg(target_os = "macos")]
        RegistrationBackend::SmAppService => macos::status(),
        #[cfg(not(target_os = "macos"))]
        RegistrationBackend::SmAppService => Err("background_service_requires_macos".to_string()),
        RegistrationBackend::SupervisedProcess => Ok(if state::requested(engine)? {
            "active"
        } else {
            "paused"
        }
        .to_string()),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests;
