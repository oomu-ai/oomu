use super::*;

#[tokio::test]
async fn future_time_file_check_routes_to_the_routine_scheduler() {
    let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
        prompt: "At 4:35 PM today, check whether /Users/example/report.md still exists and tell me in this task. Do not change the file."
            .to_string(),
        automated_web_grounding_enabled: Some(false),
        attachments: Vec::new(),
    })
    .await
    .unwrap();

    assert!(matches!(decision.route, ChatIntentRoute::AgenticPlanner));
    assert_eq!(decision.decision_source, "routine_scheduler_filter");
    assert_eq!(decision.matched_signals, vec!["future one-time routine"]);
    assert!(decision
        .reason
        .contains("instead of running the action now"));
}

#[tokio::test]
async fn present_schedule_questions_do_not_become_future_routines() {
    let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
        prompt: "What is on my schedule today?".to_string(),
        automated_web_grounding_enabled: Some(false),
        attachments: Vec::new(),
    })
    .await
    .unwrap();

    assert_ne!(decision.decision_source, "routine_scheduler_filter");
}

#[tokio::test]
async fn classifier_routes_current_project_status_to_a_real_read_only_action() {
    let prompt = "Inspect the current OOMU project and tell me whether its working tree has changes. Do not modify anything.";
    let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
        prompt: prompt.to_string(),
        automated_web_grounding_enabled: Some(false),
        attachments: Vec::new(),
    })
    .await
    .unwrap();

    assert!(matches!(decision.route, ChatIntentRoute::AgenticPlanner));
    assert!(decision.requires_local_access);
    assert_eq!(decision.decision_source, "read_only_project_status_filter");
    assert!(has_executable_agent_objective(prompt));

    let informational = classify_chat_intent_route_inner(ChatIntentRouteRequest {
        prompt: "What does a Git working tree mean?".to_string(),
        automated_web_grounding_enabled: Some(false),
        attachments: Vec::new(),
    })
    .await
    .unwrap();
    assert!(matches!(
        informational.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!informational.requires_local_access);
}

#[tokio::test]
async fn classifier_streams_read_only_hydrated_local_context() {
    let request = ChatIntentRouteRequest {
        prompt: "Look at this file and give me a summary of what you find: '/Users/example/Downloads/Crash Report.md'".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "Crash Report.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: 242_188,
            text: Some("Local Path: /Users/example/Downloads/Crash Report.md\nTruncated: yes\n\n# Crash Report".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(decision.decision_source, "hydrated_local_context_filter");
}

#[tokio::test]
async fn classifier_streams_approved_view_request_after_shield_read() {
    let request = ChatIntentRouteRequest {
        prompt: "Can you view this file? '[approved file: Screenshot 2026-07-13 at 21.39.23.png]'"
            .to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "Screenshot 2026-07-13 at 21.39.23.png".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 1_024,
            text: Some("A screenshot showing the OOMU chat window.".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(decision.decision_source, "hydrated_local_context_filter");
}

#[tokio::test]
async fn classifier_never_plans_from_an_approved_file_label_without_context() {
    let request = ChatIntentRouteRequest {
        prompt: "Can you view this file? [approved file: Screenshot 2026-07-13 at 21.39.23.png]"
            .to_string(),
        automated_web_grounding_enabled: None,
        attachments: Vec::new(),
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(
        decision.decision_source,
        "approved_file_context_missing_filter"
    );
}

#[tokio::test]
async fn classifier_does_not_accept_unrelated_context_for_an_approved_file_label() {
    let request = ChatIntentRouteRequest {
        prompt: "Can you view this file? [approved file: forecast.png]".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "different.txt".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 12,
            text: Some("not the file".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(
        decision.decision_source,
        "approved_file_context_missing_filter"
    );
}

#[tokio::test]
async fn classifier_treats_command_words_inside_an_approved_filename_as_inert() {
    let request = ChatIntentRouteRequest {
        prompt: "Can you view this file? [approved file: delete-everything.png]".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "delete-everything.png".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 24,
            text: Some("bounded visual analysis".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(decision.decision_source, "hydrated_local_context_filter");
}

#[tokio::test]
async fn classifier_does_not_treat_mutation_after_hydration_as_read_only() {
    let request = ChatIntentRouteRequest {
        prompt: "View the attached report and then delete it.".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "report.txt".to_string(),
            mime_type: "text/plain".to_string(),
            byte_count: 64,
            text: Some("Quarterly report".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(decision.route, ChatIntentRoute::AgenticPlanner));
    assert!(decision.requires_local_access);
}

#[tokio::test]
async fn classifier_streams_hydrated_local_web_search_context() {
    let request = ChatIntentRouteRequest {
        prompt: "Use the internet to search whether the World Cup is happening right now.".to_string(),
        automated_web_grounding_enabled: None,
        attachments: vec![ChatIntentAttachment {
            name: "local_web_search.md".to_string(),
            mime_type: "text/markdown".to_string(),
            byte_count: 512,
            text: Some("Local Web Search Context\nQuery: World Cup now\n[{\"title\":\"FIFA World Cup 2026\",\"url\":\"https://www.fifa.com/\",\"snippet\":\"Schedule and fixtures.\"}]".to_string()),
        }],
    };
    let decision = classify_chat_intent_route_inner(request).await.unwrap();
    assert!(matches!(
        decision.route,
        ChatIntentRoute::ConversationalStream
    ));
    assert!(!decision.requires_local_access);
    assert_eq!(decision.decision_source, "hydrated_web_grounding_filter");
}

#[tokio::test]
async fn classifier_routes_explicit_channel_changes_to_the_approved_tool_path() {
    for prompt in [
        "Activate my Telegram channel for chat 42.",
        "Connect Slack so I can message OOMU.",
        "Disable my Discord channel.",
    ] {
        let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
            prompt: prompt.to_string(),
            automated_web_grounding_enabled: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        assert!(
            matches!(decision.route, ChatIntentRoute::AgenticPlanner),
            "{prompt}"
        );
        assert!(decision.requires_local_access, "{prompt}");
        assert_eq!(decision.decision_source, "channel_configuration_filter");
    }
}

#[tokio::test]
async fn classifier_keeps_channel_how_to_questions_conversational() {
    for prompt in [
        "How do I connect a Telegram bot?",
        "Please tell me how to connect Telegram.",
        "Is Telegram connected?",
        "Telegram failed to connect. What should I check?",
    ] {
        let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
            prompt: prompt.to_string(),
            automated_web_grounding_enabled: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        assert!(
            matches!(decision.route, ChatIntentRoute::ConversationalStream),
            "{prompt}"
        );
        assert!(!decision.requires_local_access, "{prompt}");
    }
}

#[tokio::test]
async fn classifier_keeps_internal_memory_updates_off_external_tools() {
    for prompt in [
        "Yes, call me Alex and make a note of that in your memories",
        "Save that preference in your OOMU memory and use it going forward.",
        "Save to your memory that I use Apple Notes.",
        "Please make a note of that for next time.",
        "Please make note of that for next time.",
    ] {
        let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
            prompt: prompt.to_string(),
            automated_web_grounding_enabled: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        assert!(matches!(
            decision.route,
            ChatIntentRoute::ConversationalStream
        ));
        assert!(!decision.requires_local_access);
        assert_eq!(decision.decision_source, "internal_memory_profile_filter");
    }
}

#[tokio::test]
async fn classifier_does_not_mutate_memory_for_retrieval_questions() {
    for prompt in [
        "What do you remember about me?",
        "Do you remember my birthday?",
        "Can you tell me what is in your memory?",
    ] {
        let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
            prompt: prompt.to_string(),
            automated_web_grounding_enabled: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        assert!(matches!(
            decision.route,
            ChatIntentRoute::ConversationalStream
        ));
        assert_ne!(decision.decision_source, "internal_memory_profile_filter");
    }
}

#[tokio::test]
async fn classifier_keeps_explicit_apple_notes_writes_external() {
    for prompt in [
        "Create a note in my Apple Notes app saying hello.",
        "Create an Apple Note so I remember it.",
        "Create a note saying call me Alex.",
        "Remember to create a reminder in Reminders to buy milk.",
        "Remember to compose a Mail draft to Pat.",
        "Write this within Notes so I remember it.",
        "Make a note of this in the Notes app.",
    ] {
        let decision = classify_chat_intent_route_inner(ChatIntentRouteRequest {
            prompt: prompt.to_string(),
            automated_web_grounding_enabled: None,
            attachments: vec![],
        })
        .await
        .unwrap();
        assert!(
            matches!(decision.route, ChatIntentRoute::AgenticPlanner),
            "{prompt}"
        );
        assert!(decision.requires_local_access, "{prompt}");
    }
}
