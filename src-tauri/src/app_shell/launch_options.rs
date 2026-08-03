use serde::Serialize;

const NATIVE_ACCEPTANCE_SCOPE_OPTION: &str = "--native-acceptance-scope";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeAcceptanceLaunchScope {
    pub(crate) run_id: String,
    pub(crate) incident_prompt_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OomuLaunchOptions {
    pub debug_mode: bool,
    pub safe_mode: bool,
    pub first_run_setup: bool,
    pub log_level: String,
    pub dump_db: bool,
    pub reset_state: bool,
    #[serde(skip)]
    pub show_help: bool,
    #[serde(skip)]
    pub(crate) native_acceptance_scope: Option<NativeAcceptanceLaunchScope>,
    #[serde(skip)]
    pub(crate) native_acceptance_scope_error: Option<String>,
}

impl Default for OomuLaunchOptions {
    fn default() -> Self {
        Self {
            debug_mode: false,
            safe_mode: false,
            first_run_setup: false,
            log_level: "info".to_string(),
            dump_db: false,
            reset_state: false,
            show_help: false,
            native_acceptance_scope: None,
            native_acceptance_scope_error: None,
        }
    }
}

pub fn parse_launch_options() -> OomuLaunchOptions {
    parse_launch_options_from(std::env::args().skip(1))
}

pub(super) fn parse_launch_options_from<I, S>(args: I) -> OomuLaunchOptions
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect::<Vec<_>>();
    let mut options = OomuLaunchOptions::default();
    let mut log_level_explicit = false;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].trim();
        match argument {
            "--debug" | "-d" => {
                options.debug_mode = true;
                if !log_level_explicit {
                    options.log_level = "trace".to_string();
                }
            }
            "--safe-mode" | "-s" => options.safe_mode = true,
            "--first-run-setup" => options.first_run_setup = true,
            "--reset-state" => options.reset_state = true,
            "--help" | "-h" => options.show_help = true,
            NATIVE_ACCEPTANCE_SCOPE_OPTION => {
                let value = args.get(index + 1).map(String::as_str);
                set_native_acceptance_scope(&mut options, value);
                if value.is_some() {
                    index += 1;
                }
            }
            "--dump-db" | "--audit-db" => options.dump_db = true,
            "--log-level" | "-l" => {
                if let Some(level) = args
                    .get(index + 1)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty() && !value.starts_with('-'))
                    .and_then(normalize_log_level)
                {
                    options.log_level = level.to_string();
                    log_level_explicit = true;
                    index += 1;
                }
            }
            _ => {
                if let Some(value) = argument.strip_prefix("--native-acceptance-scope=") {
                    set_native_acceptance_scope(&mut options, Some(value));
                } else if let Some(level) = argument
                    .strip_prefix("--log-level=")
                    .or_else(|| argument.strip_prefix("-l="))
                    .and_then(normalize_log_level)
                {
                    options.log_level = level.to_string();
                    log_level_explicit = true;
                }
            }
        }
        index += 1;
    }
    options
}

fn set_native_acceptance_scope(options: &mut OomuLaunchOptions, value: Option<&str>) {
    if options.native_acceptance_scope.is_some() || options.native_acceptance_scope_error.is_some()
    {
        options.native_acceptance_scope = None;
        options.native_acceptance_scope_error =
            Some("The native acceptance receipt scope may be supplied only once.".to_string());
        return;
    }
    match value.and_then(parse_native_acceptance_scope) {
        Some(scope) => options.native_acceptance_scope = Some(scope),
        None => {
            options.native_acceptance_scope_error =
                Some("The native acceptance receipt scope is incomplete or invalid.".to_string())
        }
    }
}

fn parse_native_acceptance_scope(value: &str) -> Option<NativeAcceptanceLaunchScope> {
    let (run_id, incident_prompt_sha256) = value.split_once(':')?;
    let valid_run_id = (8..=64).contains(&run_id.len())
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    let valid_digest = incident_prompt_sha256.len() == 64
        && incident_prompt_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'));
    (valid_run_id && valid_digest).then(|| NativeAcceptanceLaunchScope {
        run_id: run_id.to_string(),
        incident_prompt_sha256: incident_prompt_sha256.to_string(),
    })
}

pub(crate) fn print_launch_help() {
    println!(
        "\
OOMU command-line options

Usage:
  OOMU [options]

Options:
  -h, --help                 Show this help text and exit.
  -d, --debug                Enable debug mode and default logging to trace.
  -s, --safe-mode            Launch without allowing any capability mods.
      --first-run-setup      Show the first-run setup flow for testing.
  -l, --log-level <level>    Set log level: trace, debug, info, warn, error, or off.
      --dump-db              Print a read-only state database diagnostic dump and exit.
      --audit-db             Alias for --dump-db.
      --reset-state          Purge transient queues, runtime logs, and temp cache tables before boot.
"
    );
}

fn normalize_log_level(level: &str) -> Option<&'static str> {
    match level.trim().to_ascii_lowercase().as_str() {
        "trace" => Some("trace"),
        "debug" => Some("debug"),
        "info" => Some("info"),
        "warn" | "warning" => Some("warn"),
        "error" => Some("error"),
        "off" => Some("off"),
        _ => None,
    }
}
