use super::{
    agent_manager::AgentManager, scenario_one_e2e_profile, sovereign_identity::SovereignIdentity,
    OomuError, OomuLaunchOptions,
};

#[path = "sprint_294_isolated_profile.rs"]
pub(crate) mod sprint_294_isolated_profile;

#[path = "single_instance.rs"]
mod single_instance;

pub(super) struct StartupAuthority {
    _lease: single_instance::InstanceLease,
    identity: super::single_instance_contract::InstanceIdentity,
    splash: Option<super::startup_splash::StartupSplash>,
}

impl StartupAuthority {
    pub(super) fn identity(&self) -> super::single_instance_contract::InstanceIdentity {
        self.identity.clone()
    }

    pub(super) fn require_gui_splash(&self) -> super::startup_splash::StartupSplash {
        self.splash
            .clone()
            .expect("a primary GUI launch owns startup presentation")
    }
}

pub(super) fn exit_after_help_if_requested(options: &OomuLaunchOptions) {
    if options.show_help {
        super::app_shell::print_launch_help();
        std::process::exit(0);
    }
}

pub(super) fn establish_startup_authority(
    options: &OomuLaunchOptions,
) -> Result<Option<StartupAuthority>, OomuError> {
    let claim = claim_single_instance()?;
    let splash = should_present_startup_splash(options, claim.is_primary())
        .then(super::startup_splash::StartupSplash::present);
    let process = validate_scenario_profile()?;
    let Some(lease) = complete_single_instance(claim, &process)? else {
        return Ok(None);
    };
    let identity = lease.identity().clone();
    write_runtime_profile_receipt(&process, &identity.namespace)?;
    Ok(Some(StartupAuthority {
        _lease: lease,
        identity,
        splash,
    }))
}

fn should_present_startup_splash(options: &OomuLaunchOptions, primary_claim: bool) -> bool {
    primary_claim && !options.show_help && !options.dump_db
}

fn claim_single_instance() -> Result<single_instance::InstanceClaim, OomuError> {
    single_instance::claim().map_err(|failure| OomuError::StartupIntegrity {
        code: failure.code,
        detail: failure.detail,
    })
}

fn complete_single_instance(
    claim: single_instance::InstanceClaim,
    process: &super::macos_process_identity::MacosProcessIdentityEvidence,
) -> Result<Option<single_instance::InstanceLease>, OomuError> {
    let profile_class = super::runtime_profile::current_class(process).map_err(|failure| {
        OomuError::StartupIntegrity {
            code: failure.code,
            detail: failure.detail,
        }
    })?;
    let identity =
        super::single_instance_contract::InstanceIdentity::from_process(process, profile_class);
    single_instance::complete(claim, identity).map_err(|failure| OomuError::StartupIntegrity {
        code: failure.code,
        detail: failure.detail,
    })
}

#[cfg(target_os = "macos")]
pub(super) fn install_single_instance_activation_listener(
    app: &tauri::App,
    identity: super::single_instance_contract::InstanceIdentity,
) -> Result<(), String> {
    use tauri::Manager;
    let runtime = single_instance::install_activation_listener(app.handle().clone(), identity)?;
    app.manage(runtime);
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn shutdown_single_instance_activation_listener(
    app: &tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Manager;
    app.try_state::<single_instance::SingleInstanceActivationRuntime>()
        .map_or(Ok(()), |runtime| runtime.shutdown())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn shutdown_single_instance_activation_listener(
    _app: &tauri::AppHandle,
) -> Result<(), String> {
    Ok(())
}

pub(super) fn configure_launch_logging_profile(options: &OomuLaunchOptions) {
    std::env::set_var("OOMU_LOG_LEVEL", &options.log_level);
    std::env::set_var("RUST_LOG", &options.log_level);
    std::env::set_var(
        "OOMU_DEBUG_MODE",
        if options.debug_mode { "1" } else { "0" },
    );
}

pub(super) fn validate_scenario_profile(
) -> Result<super::macos_process_identity::MacosProcessIdentityEvidence, OomuError> {
    emit_startup_identity_boundary("started");
    let process_identity = super::macos_process_identity::current();
    emit_startup_identity_boundary("completed");
    let isolated_root = std::env::var_os(super::settings::APP_DATA_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);
    let qualification_requested = sprint_294_isolated_profile::requested();
    super::runtime_profile::validate_request(
        &process_identity,
        isolated_root.is_some(),
        qualification_requested,
    )
    .map_err(|failure| OomuError::StartupIntegrity {
        code: failure.code,
        detail: failure.detail,
    })?;
    sprint_294_isolated_profile::activate(
        super::settings::APP_DATA_ROOT_ENV,
        isolated_root.as_deref(),
    )
    .map_err(OomuError::Startup)?;
    scenario_one_e2e_profile::validate_activation(
        super::settings::APP_DATA_ROOT_ENV,
        isolated_root.as_deref(),
    )
    .map_err(OomuError::Startup)?;
    if scenario_one_e2e_profile::enabled() && !sprint_294_isolated_profile::is_active() {
        return Err(OomuError::Startup(
            "Scenario qualification requires a validated isolated profile. Nothing was opened or changed."
                .to_string(),
        ));
    }
    super::keychain_namespace::bind_verified_process_identity(&process_identity).map_err(
        |detail| OomuError::StartupIntegrity {
            code: "keychain_namespace_binding_failed",
            detail: detail.to_string(),
        },
    )?;
    Ok(process_identity)
}

fn write_runtime_profile_receipt(
    identity: &super::macos_process_identity::MacosProcessIdentityEvidence,
    instance_namespace: &str,
) -> Result<(), OomuError> {
    let receipt = super::runtime_profile::receipt(identity, instance_namespace.to_string())
        .map_err(|failure| OomuError::StartupIntegrity {
            code: failure.code,
            detail: failure.detail,
        })?;
    let encoded = serde_json::to_string(&receipt).map_err(|error| {
        OomuError::Startup(format!(
            "OOMU could not encode its runtime receipt: {error}"
        ))
    })?;
    super::runtime_profile::write_receipt(&super::settings::app_data_root(), &receipt)
        .map_err(OomuError::Startup)?;
    eprintln!("OOMU_NATIVE_RECEIPT {encoded}");
    Ok(())
}

fn emit_startup_identity_boundary(state: &str) {
    let at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    eprintln!(
        "OOMU_STARTUP_MILESTONE milestone=identity_verification_{state} at_unix_ms={at_unix_ms}"
    );
}

pub(super) fn opener_plugin<R: tauri::Runtime>() -> impl tauri::plugin::Plugin<R> {
    tauri_plugin_opener::Builder::new()
        .open_js_links_on_click(false)
        .build()
}

pub(crate) fn vault_root(app_data_root: &std::path::Path) -> Option<std::path::PathBuf> {
    sprint_294_isolated_profile::knowledge_vault_root()
        .or_else(|| scenario_one_e2e_profile::vault(app_data_root))
}

pub(super) fn initialize_sovereign_identity() -> Result<SovereignIdentity, OomuError> {
    if let Some(passphrase) = sprint_294_isolated_profile::identity_passphrase() {
        return SovereignIdentity::initialize_with_session_passphrase(passphrase.as_str())
            .map_err(|error| OomuError::Startup(error.message));
    }
    match scenario_one_e2e_profile::identity_passphrase() {
        Some(passphrase) => {
            SovereignIdentity::initialize_with_session_passphrase(passphrase.as_str())
                .map_err(|error| OomuError::Startup(error.message))
        }
        None => Ok(SovereignIdentity::initialize_interactive()),
    }
}

pub(super) fn configure_scenario_agent_manager(
    agent_manager: &AgentManager,
) -> Result<(), OomuError> {
    #[cfg(debug_assertions)]
    if scenario_one_e2e_profile::enabled() {
        let updated = agent_manager
            .configure_scenario_one_e2e_model()
            .map_err(OomuError::Startup)?;
        eprintln!(
            "OOMU_SCENARIO_ONE_E2E_PROFILE local_model={} aligned_agents={updated}",
            scenario_one_e2e_profile::LOCAL_MODEL_ID
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = agent_manager;
    Ok(())
}

#[cfg(test)]
mod startup_authority_tests {
    use super::*;

    #[test]
    fn only_a_primary_gui_claim_may_present_startup_ui() {
        let gui = OomuLaunchOptions::default();
        assert!(should_present_startup_splash(&gui, true));
        assert!(!should_present_startup_splash(&gui, false));

        let mut help = gui.clone();
        help.show_help = true;
        assert!(!should_present_startup_splash(&help, true));

        let mut dump = gui.clone();
        dump.dump_db = true;
        assert!(!should_present_startup_splash(&dump, true));

        let mut reset = gui;
        reset.reset_state = true;
        assert!(should_present_startup_splash(&reset, true));
    }

    #[test]
    fn primary_claim_and_splash_remain_before_full_identity_verification() {
        let source = include_str!("launch_startup.rs");
        let claim = source.find("let claim = claim_single_instance()?").unwrap();
        let splash = source
            .find("then(super::startup_splash::StartupSplash::present)")
            .unwrap();
        let identity = source
            .find("let process = validate_scenario_profile()?")
            .unwrap();

        assert!(claim < splash);
        assert!(splash < identity);
    }
}
