use super::*;

async fn classify_schedule(prompt: &str) -> ChatIntentRouteDecision {
    classify_chat_intent_route_inner(ChatIntentRouteRequest {
        prompt: prompt.to_string(),
        automated_web_grounding_enabled: Some(false),
        attachments: Vec::new(),
    })
    .await
    .unwrap()
}

type RecurringCase = (&'static str, u32, &'static str, &'static str, bool);
const RECURRING_CASES: &[RecurringCase] = &[
    (
        "Check unread email every 5 minutes.",
        5,
        "minute",
        "every 5 minutes",
        false,
    ),
    (
        "Check unread email once per hour.",
        1,
        "hour",
        "every 1 hour",
        false,
    ),
    (
        "Schedule a daily unread email check.",
        1,
        "day",
        "every day",
        true,
    ),
    (
        "Check unread email every other day.",
        2,
        "day",
        "every 2 days",
        true,
    ),
    (
        "Check unread email every 2 weeks.",
        2,
        "week",
        "every 2 weeks",
        true,
    ),
    (
        "Schedule a fortnightly unread email check.",
        2,
        "week",
        "every 2 weeks",
        true,
    ),
    (
        "Check unread email every weekday.",
        1,
        "week",
        "every weekday",
        true,
    ),
    (
        "Check unread email weekends.",
        1,
        "week",
        "every weekend",
        true,
    ),
    (
        "Check unread email every Monday.",
        1,
        "week",
        "every monday",
        true,
    ),
    (
        "Check unread email each morning.",
        1,
        "day",
        "every morning",
        true,
    ),
    (
        "Check unread email every afternoon.",
        1,
        "day",
        "every afternoon",
        true,
    ),
    (
        "Check unread email every evening.",
        1,
        "day",
        "every evening",
        true,
    ),
    ("Check unread email nightly.", 1, "day", "every night", true),
    (
        "Schedule a monthly unread email check.",
        1,
        "month",
        "every month",
        true,
    ),
    (
        "Schedule a quarterly unread email check.",
        1,
        "quarter",
        "every quarter",
        true,
    ),
    (
        "Schedule an annual unread email check.",
        1,
        "year",
        "every year",
        true,
    ),
    (
        "Check unread email every eleven years.",
        11,
        "year",
        "every 11 years",
        true,
    ),
];

const UNSUPPORTED_CASES: &[(&str, &str, &str)] = &[
    (
        "Check unread email every 30 seconds.",
        "every 30 seconds",
        "sub-minute",
    ),
    (
        "Check unread email periodically.",
        "periodically",
        "concrete interval",
    ),
    (
        "Check unread email biweekly.",
        "biweekly",
        "more than one common meaning",
    ),
    (
        "Check unread email bimonthly.",
        "bimonthly",
        "more than one common meaning",
    ),
];

#[tokio::test]
async fn hourly_mail_check_with_immediate_run_routes_to_review_without_claiming_execution() {
    let decision =
        classify_schedule("Schedule an hourly check for unread email and run it once now.").await;
    assert!(matches!(decision.route, ChatIntentRoute::AgenticPlanner));
    assert_eq!(decision.decision_source, "routine_scheduler_filter");
    assert_eq!(
        decision.matched_signals,
        vec![
            "recurring routine",
            "routine cadence:v1:1:hour",
            "routine schedule seed: every 1 hour",
            "routine target private app:v1:mail",
            "explicit run once requested",
        ]
    );
    assert!(decision.reason.contains("remains unexecuted"));
}

#[tokio::test]
async fn recurring_mail_check_preserves_enforced_midnight_boundary_for_review() {
    let decision = classify_schedule(
        "Check my unread email every hour until midnight. Once you set it up, run it once to ensure it’s working properly.",
    )
    .await;
    assert_eq!(
        decision.matched_signals,
        vec![
            "recurring routine",
            "routine cadence:v1:1:hour",
            "routine schedule seed: every 1 hour",
            "routine target private app:v1:mail",
            "explicit run once requested",
            "end at midnight requested",
        ]
    );
    assert!(decision.reason.contains("enforced midnight stop"));
}

#[tokio::test]
async fn cadence_adjective_on_an_ordinary_noun_does_not_create_a_routine() {
    for prompt in [
        "Check my hourly rate and tell me whether it changed.",
        "Prepare a quarterly program update and create a results table.",
        "Create a monthly report as a Word document.",
    ] {
        let decision = classify_schedule(prompt).await;
        assert_ne!(
            decision.decision_source, "routine_scheduler_filter",
            "{prompt}"
        );
    }
}

#[tokio::test]
async fn coarse_cadence_ending_today_keeps_the_conflict_visible() {
    let decision = classify_schedule(
        "Check my unread email every week from now until midnight today. Once you set it up, run it once to ensure it’s working properly.",
    )
    .await;
    assert_eq!(decision.decision_source, "routine_scheduler_filter");
    assert!(decision
        .matched_signals
        .contains(&"routine cadence:v1:1:week".to_string()));
    assert!(decision
        .matched_signals
        .contains(&"routine timing defaulted".to_string()));
    assert!(decision.reason.contains("no future recurrence"));
    assert!(decision.reason.contains("cannot claim a recurring run"));
}

#[tokio::test]
async fn recurring_timeframes_share_one_typed_schedule_contract() {
    for &(prompt, interval, unit, seed, timing_defaulted) in RECURRING_CASES {
        let decision = classify_schedule(prompt).await;
        assert_eq!(
            decision.decision_source, "routine_scheduler_filter",
            "{prompt}"
        );
        assert_eq!(decision.matched_signals[0], "recurring routine", "{prompt}");
        assert!(
            decision
                .matched_signals
                .contains(&format!("routine cadence:v1:{interval}:{unit}")),
            "{prompt}"
        );
        assert!(
            decision
                .matched_signals
                .contains(&format!("routine schedule seed: {seed}")),
            "{prompt}"
        );
        assert!(
            decision
                .matched_signals
                .contains(&"routine target private app:v1:mail".to_string()),
            "{prompt}"
        );
        assert_eq!(
            decision
                .matched_signals
                .contains(&"routine timing defaulted".to_string()),
            timing_defaulted,
            "{prompt}",
        );
        assert!(
            !decision
                .matched_signals
                .contains(&"routine schedule unsupported".to_string()),
            "{prompt}"
        );
    }
}

#[tokio::test]
async fn unsupported_cadences_open_truthful_clarification_review() {
    for &(prompt, seed, reason) in UNSUPPORTED_CASES {
        let decision = classify_schedule(prompt).await;
        assert_eq!(
            decision.decision_source, "routine_scheduler_filter",
            "{prompt}"
        );
        assert_eq!(
            decision.matched_signals,
            vec![
                "recurring routine".to_string(),
                format!("routine schedule seed: {seed}"),
                "routine schedule unsupported".to_string(),
                "routine schedule clarification required".to_string(),
                "routine target private app:v1:mail".to_string(),
            ]
        );
        assert!(decision.reason.contains(reason), "{prompt}");
    }
}
