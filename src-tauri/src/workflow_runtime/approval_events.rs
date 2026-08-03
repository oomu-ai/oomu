use super::ApprovalRequest;
use tauri::Emitter;

const APPROVAL_REQUESTED_EVENT: &str = "workflow://approval-requested";

fn approval_event(request: Option<&ApprovalRequest>) -> Option<(&'static str, &ApprovalRequest)> {
    request.map(|request| (APPROVAL_REQUESTED_EVENT, request))
}

pub(crate) fn dispatch_approval_request(app: &tauri::AppHandle, request: Option<&ApprovalRequest>) {
    let Some((event, request)) = approval_event(request) else {
        return;
    };
    if let Err(error) = app.emit(event, request) {
        eprintln!(
            "WORKFLOW_APPROVAL_NOTIFICATION_FAILED instance_id={} error={}",
            request.instance_id, error
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn interactive_and_scheduled_runs_share_the_existing_approval_event_contract() {
        let request = ApprovalRequest {
            instance_id: "interactive-instance".to_string(),
            workflow_id: "interactive-workflow".to_string(),
            node_id: "permission".to_string(),
            message: "Approve?".to_string(),
            context: json!({"kind":"calendar"}),
            approval_token: "exact-token".to_string(),
            approve_command: json!({"decision":"approve"}),
            reject_command: json!({"decision":"reject"}),
        };
        let (event, payload) = approval_event(Some(&request)).expect("approval event");
        assert_eq!(event, "workflow://approval-requested");
        assert_eq!(payload.instance_id, "interactive-instance");
        assert_eq!(payload.approval_token, "exact-token");
        assert!(approval_event(None).is_none());
    }
}
