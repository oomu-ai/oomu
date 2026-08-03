use std::{
    fmt,
    io::{self, Write},
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct FunctionalAcceptanceConfig {
    run_id: String,
    incident_prompt_sha256: String,
}

impl FunctionalAcceptanceConfig {
    fn from_launch_scope(scope: &crate::app_shell::NativeAcceptanceLaunchScope) -> Self {
        Self {
            run_id: scope.run_id.clone(),
            incident_prompt_sha256: scope.incident_prompt_sha256.clone(),
        }
    }
}

#[derive(Default)]
struct FunctionalAcceptanceScope {
    config: Option<FunctionalAcceptanceConfig>,
    session_id: Option<String>,
}

impl FunctionalAcceptanceScope {
    fn permits(
        &mut self,
        config: &FunctionalAcceptanceConfig,
        receipt: &serde_json::Value,
    ) -> bool {
        if self.config.as_ref() != Some(config) {
            self.config = Some(config.clone());
            self.session_id = None;
        }
        let Some(session_id) = receipt
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return false;
        };
        if let Some(bound_session_id) = self.session_id.as_deref() {
            return bound_session_id == session_id;
        }
        let binds_scope = receipt.get("kind").and_then(serde_json::Value::as_str)
            == Some("accepted_chat_turn")
            && receipt
                .get("messageSha256")
                .and_then(serde_json::Value::as_str)
                == Some(config.incident_prompt_sha256.as_str());
        if binds_scope {
            self.session_id = Some(session_id.to_string());
        }
        binds_scope
    }
}

static FUNCTIONAL_ACCEPTANCE_SCOPE: OnceLock<Mutex<FunctionalAcceptanceScope>> = OnceLock::new();
static FUNCTIONAL_ACCEPTANCE_CONFIG: OnceLock<FunctionalAcceptanceConfig> = OnceLock::new();

pub(crate) fn configure_native_acceptance(
    options: &crate::OomuLaunchOptions,
) -> Result<(), String> {
    if let Some(error) = options.native_acceptance_scope_error.as_deref() {
        return Err(error.to_string());
    }
    let Some(scope) = options.native_acceptance_scope.as_ref() else {
        return Ok(());
    };
    let config = FunctionalAcceptanceConfig::from_launch_scope(scope);
    if !functional_acceptance_run_id_is_valid(Some(&config.run_id))
        || !functional_acceptance_digest_is_valid(&config.incident_prompt_sha256)
    {
        return Err("The native acceptance receipt scope is invalid.".to_string());
    }
    FUNCTIONAL_ACCEPTANCE_CONFIG
        .set(config)
        .map_err(|_| "The native acceptance receipt scope was already configured.".to_string())
}

pub(crate) fn native_acceptance_enabled() -> bool {
    FUNCTIONAL_ACCEPTANCE_CONFIG.get().is_some()
}

pub(crate) fn write_diagnostic_line(arguments: fmt::Arguments<'_>) {
    let mut stderr = io::stderr().lock();
    write_diagnostic_line_to(&mut stderr, arguments);
}

pub(crate) fn write_functional_acceptance_receipt(receipt: &serde_json::Value) {
    let Some(config) = FUNCTIONAL_ACCEPTANCE_CONFIG.get() else {
        return;
    };
    let scope = FUNCTIONAL_ACCEPTANCE_SCOPE.get_or_init(Default::default);
    let Ok(mut scope) = scope.lock() else {
        return;
    };
    if scope.permits(config, receipt) {
        write_diagnostic_line(format_args!("OOMU_NATIVE_RECEIPT {receipt}"));
    }
}

fn functional_acceptance_run_id_is_valid(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        (8..=64).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn functional_acceptance_digest_is_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(crate) fn debug_trace_enabled() -> bool {
    matches!(
        std::env::var("OOMU_DEBUG_MODE").as_deref(),
        Ok("1") | Ok("true")
    )
}

pub(crate) fn prepare_launch(
    value: crate::OomuLaunchOptions,
) -> Result<crate::OomuLaunchOptions, String> {
    write_diagnostic_line(format_args!("{}", build_provenance_receipt()));
    configure_native_acceptance(&value)?;
    Ok(value)
}

fn build_provenance_receipt() -> String {
    format!(
        r#"OOMU_NATIVE_RECEIPT {{"kind":"packaged_build_identity","sourceTreeSha256":"{}","sourceFileCount":{},"frontendExportSha256":"{}","frontendExportFileCount":{}}}"#,
        env!("OOMU_BUILD_SOURCE_SHA256"),
        env!("OOMU_BUILD_SOURCE_FILE_COUNT"),
        env!("OOMU_FRONTEND_EXPORT_SHA256"),
        env!("OOMU_FRONTEND_EXPORT_FILE_COUNT")
    )
}

fn write_diagnostic_line_to<W: Write + ?Sized>(writer: &mut W, arguments: fmt::Arguments<'_>) {
    // Diagnostics are best-effort. In particular, a closed launch pipe must
    // not turn an otherwise valid provider response into a process abort.
    let _ = writeln!(writer, "{arguments}");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed pipe"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed pipe"))
        }
    }

    #[test]
    fn diagnostic_line_preserves_formatting_and_adds_one_newline() {
        let mut output = Vec::new();

        write_diagnostic_line_to(
            &mut output,
            format_args!("provider={} attempts={}", "gemini", 2),
        );

        assert_eq!(output, b"provider=gemini attempts=2\n");
    }

    #[test]
    fn broken_diagnostic_pipe_never_panics() {
        let result = std::panic::catch_unwind(|| {
            let mut writer = BrokenPipeWriter;
            write_diagnostic_line_to(&mut writer, format_args!("GEMINI_RESPONSE_METADATA"));
        });

        assert!(result.is_ok());
    }

    #[test]
    fn functional_receipts_require_an_explicit_bounded_run_id() {
        assert!(functional_acceptance_run_id_is_valid(Some("20260724_run")));
        assert!(!functional_acceptance_run_id_is_valid(None));
        assert!(!functional_acceptance_run_id_is_valid(Some("short")));
        assert!(!functional_acceptance_run_id_is_valid(Some(
            "run/with/path"
        )));
        assert!(functional_acceptance_digest_is_valid(&"a".repeat(64)));
        assert!(!functional_acceptance_digest_is_valid(&"A".repeat(64)));
        assert!(!functional_acceptance_digest_is_valid(&"a".repeat(63)));
    }

    fn functional_config(expected_digest: &str) -> FunctionalAcceptanceConfig {
        FunctionalAcceptanceConfig {
            run_id: "20260724_run".to_string(),
            incident_prompt_sha256: expected_digest.to_string(),
        }
    }

    #[test]
    fn functional_receipt_scope_ignores_mismatches_until_the_exact_incident_turn() {
        let expected_digest = "a".repeat(64);
        let config = functional_config(&expected_digest);
        let mut scope = FunctionalAcceptanceScope::default();

        assert!(!scope.permits(
            &config,
            &serde_json::json!({
                "kind": "accepted_chat_turn",
                "sessionId": "background-session",
                "messageSha256": "b".repeat(64),
            }),
        ));
        assert!(!scope.permits(
            &config,
            &serde_json::json!({
                "kind": "verified_sovereign_search",
                "sessionId": "background-session",
                "sourceUrls": ["https://private.invalid/unrelated"],
            }),
        ));
        assert!(scope.permits(
            &config,
            &serde_json::json!({
                "kind": "accepted_chat_turn",
                "sessionId": "functional-session",
                "messageSha256": expected_digest,
            }),
        ));
    }

    #[test]
    fn functional_receipt_scope_emits_only_the_bound_session_after_binding() {
        let expected_digest = "c".repeat(64);
        let config = functional_config(&expected_digest);
        let mut scope = FunctionalAcceptanceScope::default();
        assert!(scope.permits(
            &config,
            &serde_json::json!({
                "kind": "accepted_chat_turn",
                "sessionId": "functional-session",
                "messageSha256": expected_digest,
            }),
        ));

        assert!(scope.permits(
            &config,
            &serde_json::json!({
                "kind": "validated_chat_stream",
                "sessionId": "functional-session",
            }),
        ));
        assert!(!scope.permits(
            &config,
            &serde_json::json!({
                "kind": "verified_native_create_file",
                "sessionId": "background-session",
                "path": "/Users/someone/private.md",
            }),
        ));
        assert!(!scope.permits(
            &config,
            &serde_json::json!({
                "kind": "accepted_chat_turn",
                "sessionId": "second-matching-session",
                "messageSha256": expected_digest,
            }),
        ));
    }

    #[test]
    fn build_provenance_receipt_is_machine_readable() {
        let line = build_provenance_receipt();
        let payload = line.strip_prefix("OOMU_NATIVE_RECEIPT ").unwrap();
        let receipt: serde_json::Value = serde_json::from_str(payload).unwrap();

        assert_eq!(receipt["kind"], "packaged_build_identity");
        assert_eq!(receipt["sourceTreeSha256"].as_str().unwrap().len(), 64);
        assert!(receipt["sourceFileCount"].as_u64().unwrap() > 0);
        assert_eq!(receipt["frontendExportSha256"].as_str().unwrap().len(), 64);
        assert!(receipt["frontendExportFileCount"].as_u64().unwrap() > 0);
    }
}
