use super::super::output_integrity::has_repetition_collapse;
use super::super::*;

#[test]
fn repetition_collapse_detection_rejects_malformed_chat_output() {
    let collapsed = "Hello. Hello, I am OOMU. I am. Hello, am. I Hello. am. I I. I Hello. am. I am. am. am. Hello. am. I. I Hello. Hello. I. am. am. I Hello.";
    let healthy = "Hello, I am OOMU. I am a pragmatic local OOMU agent focused on helping Alex complete work clearly, securely, and efficiently.";

    assert!(has_repetition_collapse("Introduce yourself.", collapsed));
    assert!(!has_repetition_collapse("Introduce yourself.", healthy));
}

#[test]
fn requested_thirty_copy_rewrite_is_exact_and_not_treated_as_collapse() {
    let source = std::iter::repeat_n("The colour label is blue.", 30)
        .collect::<Vec<_>>()
        .join(" ");
    let expected = std::iter::repeat_n("The color label is blue.", 30)
        .collect::<Vec<_>>()
        .join(" ");
    let prompt = format!(
        "{source} Replace every occurrence of colour with color. Make no other change and do not explain."
    );

    assert_eq!(
        sanitize_gemma4_response_for_prompt(&expected, Some(&prompt)),
        expected
    );
    assert!(!has_repetition_collapse(&prompt, &expected));
    let unrequested = std::iter::repeat_n("I am stuck in a loop.", 30)
        .collect::<Vec<_>>()
        .join(" ");
    assert!(has_repetition_collapse(&prompt, &unrequested));

    let formatted_prompt = format_gemma4_chat_prompt(
        "Answer only the latest request.",
        &[("user".to_string(), prompt)],
    );
    assert_eq!(
        sanitize_gemma4_response_for_prompt(
            "The model produced the wrong count.",
            Some(&formatted_prompt),
        ),
        expected
    );
}

#[test]
fn disabled_service_executes_thirty_copy_rewrite_without_model_or_stream() {
    let source = std::iter::repeat_n("The colour label is blue.", 30)
        .collect::<Vec<_>>()
        .join(" ");
    let expected = std::iter::repeat_n("The color label is blue.", 30)
        .collect::<Vec<_>>()
        .join(" ");
    let user_prompt = format!(
        "{source} Replace every occurrence of colour with color. Make no other change and do not explain."
    );
    let formatted_prompt = format_gemma4_chat_prompt(
        "Answer only the latest request.",
        &[("user".to_string(), user_prompt)],
    );
    let service = GemmaService::new_disabled("model access is forbidden by this test");
    let mut stream_callbacks = 0usize;
    let mut stream = |_chunk: GemmaStreamChunk| {
        stream_callbacks += 1;
    };

    let response = service
        .infer_model_with_stream_sync(
            "model-that-must-not-load",
            InferRequest::new(formatted_prompt),
            Some(&mut stream),
        )
        .expect("the native bounded rewrite does not require a model");

    assert_eq!(response.text, expected);
    assert_eq!(response.prompt_token_count, 0);
    assert_eq!(response.generated_token_count, 0);
    assert_eq!(response.time_to_first_token_ms, 0);
    assert_eq!(stream_callbacks, 0);
    assert_eq!(
        response.model_path,
        deterministic_transform::BOUNDED_REWRITE_TRANSFORM_MODEL_PATH
    );
    assert_eq!(
        response.device,
        deterministic_transform::BOUNDED_REWRITE_TRANSFORM_DEVICE
    );
    assert!(matches!(response.service_status, GemmaStatus::Degraded));
    assert!(response
        .reasoning_trace
        .iter()
        .any(|entry| entry.contains("without loading model weights")));
    assert!(!response
        .reasoning_trace
        .iter()
        .any(|entry| entry.contains("Transformer loop generated")));
}

#[test]
fn session_only_memory_acknowledgement_honors_exact_reply_contract() {
    let prompt = concat!(
        "Remember these temporary test values for this chat only: ",
        "cedar 14, indigo 22, quartz 31. Reply stored."
    );
    assert_eq!(
        sanitize_gemma4_response_for_prompt(
            "The temporary test values have been noted for this session.",
            Some(prompt),
        ),
        "stored"
    );
    let formatted_prompt = format_gemma4_chat_prompt(
        "Answer only the latest request.",
        &[("user".to_string(), prompt.to_string())],
    );
    assert_eq!(
        sanitize_gemma4_response_for_prompt("A verbose acknowledgement.", Some(&formatted_prompt),),
        "stored"
    );
}

#[test]
fn response_removes_orphan_reserved_tags_but_preserves_complete_directives() {
    assert_eq!(
        sanitize_gemma4_response("Cedar 14\nIndigo 22\nQuartz 31\n</OomuSplitView>"),
        "Cedar 14\nIndigo 22\nQuartz 31"
    );
    assert_eq!(sanitize_gemma4_response("<OomuSplitView>stored"), "stored");
    assert_eq!(sanitize_gemma4_response("stored"), "stored");

    let directive = "<OomuSplitView><mod_id>ai.eldris.mods.browser</mod_id><action>NAVIGATE</action><url>https://example.com</url></OomuSplitView>";
    assert_eq!(sanitize_gemma4_response(directive), directive);
}
