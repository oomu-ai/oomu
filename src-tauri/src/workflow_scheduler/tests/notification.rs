use super::*;

#[test]
fn completed_empty_runs_keep_their_subtype_in_background_copy_and_receipts() {
    let copy = SchedulerCopy::english();
    let notification = background_notice_copy(
        &copy,
        "Email Responder",
        ExecutionStatus::Completed,
        Some(WorkflowCompletionKind::EmptyCollection),
        None,
        &[],
    )
    .unwrap();
    assert_eq!(notification.0, "Workflow Completed");
    assert_eq!(
        notification.1,
        "Email Responder completed. Nothing was found, so no later steps ran."
    );

    let delivery = routine_notice_copy(
        &copy,
        "Email Responder",
        ExecutionStatus::Completed,
        Some(WorkflowCompletionKind::EmptyCollection),
        None,
        None,
        &[],
    )
    .unwrap();
    assert_eq!(delivery.0, "completed_empty");
    assert_eq!(
        delivery.1,
        "Email Responder completed. Nothing was found, so no later steps ran."
    );
    assert!(!delivery.1.contains("verified result"));
}

#[test]
fn normal_completion_and_failure_copy_are_clear_and_actionable() {
    let copy = SchedulerCopy::english();
    assert_eq!(
        background_notice_copy(
            &copy,
            "Daily Brief",
            ExecutionStatus::Completed,
            None,
            None,
            &[]
        ),
        Some((
            "Workflow Completed",
            "Daily Brief completed successfully.".to_string()
        ))
    );
    assert_eq!(
        routine_notice_copy(
            &copy,
            "Daily Brief",
            ExecutionStatus::Completed,
            None,
            None,
            None,
            &[],
        ),
        Some((
            "completed",
            "Daily Brief completed. Open OOMU Tasks for the verified result.".to_string()
        ))
    );
    assert_eq!(
        background_notice_copy(
            &copy,
            "Daily Brief",
            ExecutionStatus::Failed,
            Some(WorkflowCompletionKind::EmptyCollection),
            Some("Calendar is unavailable."),
            &[],
        ),
        Some((
            "Workflow Needs Attention",
            "Daily Brief stopped before finishing: Calendar is unavailable. Open OOMU to review or retry."
                .to_string()
        ))
    );
    assert_eq!(
        routine_notice_copy(
            &copy,
            "Daily Brief",
            ExecutionStatus::Failed,
            Some(WorkflowCompletionKind::EmptyCollection),
            Some("Calendar is unavailable."),
            None,
            &[],
        ),
        Some((
            "failed",
            "Daily Brief failed. Calendar is unavailable.".to_string()
        ))
    );
}

#[test]
fn declined_actions_are_named_without_reporting_the_workflow_as_failed() {
    let copy = SchedulerCopy::english();
    let declined = vec!["Send the email".to_string()];
    assert_eq!(
        background_notice_copy(
            &copy,
            "Supplier follow-up",
            ExecutionStatus::Completed,
            None,
            None,
            &declined,
        ),
        Some((
            "Workflow Finished",
            "Supplier follow-up finished. Not performed at your request: Send the email."
                .to_string(),
        ))
    );
    assert_eq!(
        routine_notice_copy(
            &copy,
            "Supplier follow-up",
            ExecutionStatus::Completed,
            None,
            None,
            None,
            &declined,
        ),
        Some((
            "completed",
            "Supplier follow-up finished. Not performed at your request: Send the email. Open OOMU Tasks for the verified result."
                .to_string(),
        ))
    );
}
