use super::*;

#[test]
fn multimodal_projector_discovery_requires_one_real_companion() {
    let root = std::env::temp_dir().join(format!(
        "oomu-mmproj-discovery-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    fs::create_dir_all(&root).unwrap();
    let model = root.join("gemma-4-12b.gguf");
    let projector = root.join("mmproj-gemma-4-12b.gguf");
    fs::write(&model, [0_u8; 8]).unwrap();
    let projector_file = fs::File::create(&projector).unwrap();
    projector_file.set_len(1024 * 1024 + 1).unwrap();

    assert_eq!(discover_multimodal_projector(&model), Some(projector));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deterministic_compile_receipt_does_not_invent_model_identity() {
    let response = codebase_compile_response(
        CodebaseCompileTarget::Backend,
        true,
        "compile checks completed".to_string(),
        Vec::new(),
    );
    assert!(response.model_used.is_none());
}

#[test]
fn runtime_config_can_be_boosted_for_specific_model_load() {
    let base = RuntimeConfig {
        context_size: 2_048,
        batch_size: 512,
        ubatch_size: 128,
        decode_threads: 4,
        batch_threads: 8,
        requested_gpu_layers: 0,
        use_mmap: true,
        use_mlock: false,
        max_sessions: 4,
        idle_timeout_secs: 300,
        pinned_prefix_tokens: 256,
    };

    let boosted = base.with_min_context_size(Some(8_192));
    assert_eq!(boosted.context_size, 8_192);
    assert_eq!(boosted.batch_size, base.batch_size);

    let unchanged = base.with_min_context_size(Some(1_024));
    assert_eq!(unchanged.context_size, base.context_size);
}

#[test]
fn incomplete_or_non_gguf_files_are_rejected_before_native_loading() {
    let root = env::temp_dir().join(format!(
        "oomu-native-runtime-validation-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create runtime validation directory");
    let wrong_extension = root.join("model.bin");
    fs::write(&wrong_extension, b"GGUF").expect("write wrong extension");
    assert_eq!(
        validate_gguf_readiness(&wrong_extension)
            .expect_err("wrong extension must fail")
            .code,
        "llama_gguf_required"
    );

    let incomplete = root.join("model.gguf");
    fs::write(&incomplete, b"GGUF").expect("write incomplete GGUF");
    assert_eq!(
        validate_gguf_readiness(&incomplete)
            .expect_err("short GGUF must fail")
            .code,
        "llama_model_write_incomplete"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn context_shift_preserves_pinned_prefixes_and_drops_oldest_unpinned_tokens() {
    assert_eq!(
        plan_context_shift(7_500, 512, 1_000, 0, 8_191),
        ContextShiftPlan {
            cached_tokens: 309,
            incoming_tokens: 0,
        }
    );
    assert_eq!(
        plan_context_shift(0, 0, 9_000, 512, 8_191),
        ContextShiftPlan {
            cached_tokens: 0,
            incoming_tokens: 809,
        }
    );
}

#[test]
fn token_piece_sanitizer_filters_markers_split_across_native_tokens() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(sanitizer.push("Visible "), "Visible ");
    assert_eq!(sanitizer.push("<|chan"), "");
    assert_eq!(sanitizer.push("nel|>hidden"), "hidden");
    assert_eq!(sanitizer.push(" <think>reason"), " reason");
    assert_eq!(sanitizer.push("</think> answer"), " answer");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_preserves_non_protocol_angle_brackets() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(sanitizer.push("Use <section>"), "Use <section>");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_suppresses_primed_hidden_reasoning() {
    // The prompt opened the thought channel, so generation starts suppressed and only
    // the content after the model switches to `<|channel>text` is revealed. The opening
    // marker is never streamed (it lived in the prompt) and the markers arrive as
    // separate native tokens, as they do under special-token detokenization.
    let mut sanitizer = TokenPieceSanitizer::new(true, true);
    assert_eq!(sanitizer.push("The user greeted me."), "");
    assert_eq!(sanitizer.push("<|channel>"), "");
    assert_eq!(sanitizer.push("text"), "");
    assert_eq!(sanitizer.push("<channel|>"), "");
    assert_eq!(sanitizer.push("Hello, Alex."), "Hello, Alex.");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_drops_label_before_empty_channel() {
    // gemma4 12B emits the label as plain text BEFORE the markers:
    // "thought" <|channel> <channel|> answer. Drop the stray label, keep the answer.
    let mut sanitizer = TokenPieceSanitizer::new(true, false);
    assert_eq!(sanitizer.push("thought"), "");
    assert_eq!(sanitizer.push("<|channel>"), "");
    assert_eq!(sanitizer.push("<channel|>"), "");
    assert_eq!(sanitizer.push("Good"), "Good");
    assert_eq!(sanitizer.push(" morning."), " morning.");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_keeps_label_word_that_is_real_content() {
    // A label word NOT followed by a channel marker is real content and must survive.
    let mut sanitizer = TokenPieceSanitizer::new(true, false);
    assert_eq!(sanitizer.push("final"), "");
    assert_eq!(sanitizer.push(" answer: 42"), "final answer: 42");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_strips_reemitted_thought_label() {
    // Larger variants re-announce the thought channel; neither the bare "<|channel>"
    // opener nor its "thought" label may leak before the visible answer.
    let mut sanitizer = TokenPieceSanitizer::new(true, true);
    assert_eq!(sanitizer.push("<|channel>"), "");
    assert_eq!(sanitizer.push("thought"), "");
    assert_eq!(sanitizer.push("<channel|>"), "");
    assert_eq!(sanitizer.push("Reasoning..."), "");
    assert_eq!(sanitizer.push("<|channel>text<channel|>"), "");
    assert_eq!(sanitizer.push("Final answer."), "Final answer.");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_preserves_raw_whitespace_chunks() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(sanitizer.push("Hello."), "Hello.");
    assert_eq!(sanitizer.push(" "), " ");
    assert_eq!(sanitizer.push("World."), "World.");
    assert_eq!(sanitizer.push("\n"), "\n");
    assert_eq!(sanitizer.push("\nLine 2."), "\nLine 2.");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_keeps_boundaries_around_stripped_markers() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(
        sanitizer.push("here.<|channel>text<channel|>Claim"),
        "here. Claim"
    );
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_keeps_boundaries_across_marker_chunks() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(sanitizer.push("Whenever"), "Whenever");
    assert_eq!(sanitizer.push("<|channel>text"), "");
    assert_eq!(sanitizer.push("<channel|>"), "");
    assert_eq!(sanitizer.push("you go"), " you go");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
fn token_piece_sanitizer_does_not_insert_boundary_before_punctuation() {
    let mut sanitizer = TokenPieceSanitizer::default();
    assert_eq!(sanitizer.push("word<|channel>text<channel|>."), "word.");
    assert_eq!(sanitizer.finish(), "");
}

#[test]
#[ignore = "requires an installed multi-gigabyte GGUF model"]
fn installed_e2b_streams_native_token_events_and_honors_cancellation() {
    let directory = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR)
        .join("../assets/models/gemma-4-E2B-it-qat-q4_0-gguf");
    if !directory.is_dir() {
        return;
    }
    let model_path = fs::read_dir(&directory)
        .expect("read E2B model directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains("mmproj")
        })
        .expect("installed E2B text GGUF");
    let runtime = NativeRuntime::initialize().expect("initialize llama.cpp runtime");
    let (handle, _) = runtime.load_model(&model_path).expect("load E2B model");
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut streamed = String::new();
    let result = handle
        .generate(
            NativeGenerationRequest {
                session: NativeSessionRequest {
                    session_id: "phase-3-stream-test".to_string(),
                    system_prompt: None,
                    prompt: concat!(
                        "<|turn>system\nAnswer concisely.<turn|>\n",
                        "<|turn>user\nSay hello in one short sentence.<turn|>\n",
                        "<|turn>model\n<|channel>thought\n<channel|>"
                    )
                    .to_string(),
                    prompt_is_full_context: true,
                },
                media: Vec::new(),
                max_new_tokens: 16,
                temperature: 0.0,
                top_k: 40,
                top_p: 0.95,
                repeat_penalty: 1.1,
                grammar: None,
                cancellation: Arc::clone(&cancellation),
            },
            |event| streamed.push_str(&event.text),
        )
        .expect("stream native generation");
    assert!(!result.cancelled);
    assert!(!result.token_ids.is_empty());
    assert_eq!(streamed, result.text);
    eprintln!(
        "E2B_LLAMA_CPP_STREAM tokens={} ttft_ms={}",
        result.token_ids.len(),
        result.time_to_first_token_ms
    );
    assert!(
        result.time_to_first_token_ms < Duration::from_secs(12).as_millis(),
        "E2B time to first token exceeded 12 seconds"
    );

    cancellation.store(true, Ordering::Release);
    let cancelled = handle
        .generate(
            NativeGenerationRequest {
                session: NativeSessionRequest {
                    session_id: "phase-3-cancel-test".to_string(),
                    system_prompt: None,
                    prompt: "Continue indefinitely.".to_string(),
                    prompt_is_full_context: false,
                },
                media: Vec::new(),
                max_new_tokens: 64,
                temperature: 0.8,
                top_k: 40,
                top_p: 0.95,
                repeat_penalty: 1.1,
                grammar: None,
                cancellation,
            },
            |_| panic!("pre-cancelled generation must not emit tokens"),
        )
        .expect("cancel native generation");
    assert!(cancelled.cancelled);
    assert!(cancelled.token_ids.is_empty());
}

#[test]
#[ignore = "requires installed multi-gigabyte GGUF models; diagnostic"]
fn diagnose_prompt_endings_across_models() {
    let only = std::env::var("DIAG_MODEL").ok();
    let models = [
        ("E2B", "../assets/models/gemma-4-E2B-it-qat-q4_0-gguf"),
        ("E4B", "../assets/models/gemma-4-E4B-it-qat-q4_0-gguf"),
        ("12B", "../assets/models/gemma-4-12B-it-qat-q4_0-gguf"),
    ];
    let system =
        "<|turn>system\nYou are OOMU, a concise senior advisor. Do not use AI-isms.<turn|>\n";
    let history = concat!(
        "<|turn>user\nHello OOMU<turn|>\n",
        "<|turn>model\nHello Dr. Allan. How can I assist you today?<turn|>\n",
        "<|turn>user\nI just changed your model. Do you feel any different?<turn|>\n",
    );
    let endings = [
        ("bare", "<|turn>model\n"),
        ("thought", "<|turn>model\n<|channel>thought\n<channel|>"),
        ("text", "<|turn>model\n<|channel>text\n<channel|>"),
    ];
    for (label, dir) in models {
        if let Some(filter) = &only {
            if !filter.eq_ignore_ascii_case(label) {
                continue;
            }
        }
        let directory = PathBuf::from(crate::runtime_profile::OOMU_MANIFEST_DIR).join(dir);
        let Some(model_path) = find_text_gguf(&directory) else {
            eprintln!(
                "DIAG SKIP {label}: no text gguf under {}",
                directory.display()
            );
            continue;
        };
        let runtime = NativeRuntime::initialize().expect("initialize llama.cpp runtime");
        let (handle, _) = runtime.load_model(&model_path).expect("load model");
        for (ending_name, ending) in endings {
            let prompt = format!("{system}{history}{ending}");
            for (run_name, temperature) in [
                ("greedy", 0.0f32),
                ("t07_a", 0.7),
                ("t07_b", 0.7),
                ("t07_c", 0.7),
            ] {
                // Cold-start each run so cross-session KV prefix caching can't collapse the
                // identical diagnostic prompt to a zero-token decode.
                handle.flush_memory().ok();
                let cancellation = Arc::new(AtomicBool::new(false));
                match handle.generate(
                    NativeGenerationRequest {
                        session: NativeSessionRequest {
                            session_id: format!("diag-{label}-{ending_name}-{run_name}"),
                            system_prompt: None,
                            prompt: prompt.clone(),
                            prompt_is_full_context: true,
                        },
                        media: Vec::new(),
                        max_new_tokens: 160,
                        temperature,
                        top_k: 40,
                        top_p: 0.95,
                        repeat_penalty: 1.1,
                        grammar: None,
                        cancellation,
                    },
                    |_| {},
                ) {
                    Ok(result) => {
                        let visible_empty = result.text.trim().is_empty();
                        eprintln!(
                                "\n##### {label} | ending={ending_name} | {run_name} | tokens={} | visible_empty={visible_empty} #####",
                                result.token_ids.len()
                            );
                        eprintln!("RAW<<<{}>>>", result.raw_text);
                        eprintln!("SANITIZED<<<{}>>>", result.text);
                    }
                    Err(error) => {
                        eprintln!(
                            "\n##### {label} | ending={ending_name} | {run_name} | ERROR {} #####",
                            error.code
                        );
                        eprintln!("{}", error.message);
                    }
                }
            }
        }
        handle.flush_memory().ok();
    }
}
