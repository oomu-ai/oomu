use super::WorkbookStatusCode;

pub(super) fn status_code(value: &str) -> WorkbookStatusCode {
    match value {
        "building" => WorkbookStatusCode::Building,
        "ready" => WorkbookStatusCode::Ready,
        "needs_recalculation" => WorkbookStatusCode::NeedsRecalculation,
        "failed" => WorkbookStatusCode::Failed,
        _ => WorkbookStatusCode::CheckRequired,
    }
}

pub(super) fn bounded_display(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        value
            .chars()
            .take(limit.saturating_sub(1))
            .collect::<String>()
            + "…"
    }
}
