use super::format_shield_gate_halt_message;

pub(super) fn preflight_halt_message(reason: &str) -> String {
    if is_delete_target_not_found(reason) {
        return "That file is not there, so there is nothing to delete. Check the path and try again."
            .to_string();
    }
    if reason.contains("Shield Gate rejected")
        || reason.contains("security_boundary_violation")
        || reason.contains("project quarantine")
        || reason.contains("outside its safe root")
    {
        return format_shield_gate_halt_message(reason);
    }

    reason.to_string()
}

pub(super) fn preflight_error_code(reason: &str, fallback: &'static str) -> &'static str {
    if is_delete_target_not_found(reason) {
        "delete_target_not_found"
    } else {
        fallback
    }
}

fn is_delete_target_not_found(reason: &str) -> bool {
    reason.contains("delete_target_not_found")
        || reason.contains("The requested file is not there.")
}
