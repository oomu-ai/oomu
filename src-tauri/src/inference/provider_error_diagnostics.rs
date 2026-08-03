use super::redact_provider_error_text;

pub(super) const MAX_PROVIDER_ERROR_LOG_CHARS: usize = 512;

pub(super) fn bounded_provider_error_log_detail(message: &str) -> String {
    let redacted = redact_provider_error_text(message);
    let mut characters = redacted.chars();
    let bounded = characters
        .by_ref()
        .take(MAX_PROVIDER_ERROR_LOG_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}
