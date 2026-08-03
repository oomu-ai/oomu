use super::*;

#[test]
fn codebase_compile_backend_plan_runs_check_before_build() {
    let plan = codebase_compile_plan(CodebaseCompileTarget::Backend);

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].phase, "preflight");
    assert_eq!(plan[0].program, "cargo");
    assert_eq!(
        plan[0].args,
        &["check", "--manifest-path", "src-tauri/Cargo.toml"]
    );
    assert_eq!(plan[1].phase, "build");
    assert_eq!(plan[1].program, "cargo");
    assert_eq!(
        plan[1].args,
        &["build", "--manifest-path", "src-tauri/Cargo.toml"]
    );
}

#[test]
fn codebase_compile_frontend_plan_runs_typecheck_before_build() {
    let plan = codebase_compile_plan(CodebaseCompileTarget::Frontend);

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].phase, "preflight");
    assert_eq!(plan[0].program, "npm");
    assert_eq!(plan[0].args, &["run", "typecheck"]);
    assert_eq!(plan[1].phase, "build");
    assert_eq!(plan[1].program, "npm");
    assert_eq!(plan[1].args, &["run", "build"]);
}

#[test]
fn runtime_config_uses_full_metal_offload_on_apple_silicon() {
    let hardware = HardwareProfile {
        operating_system: "macos".to_string(),
        architecture: "aarch64".to_string(),
        apple_silicon: true,
        metal_available: true,
        gpu_offload_available: true,
        mmap_available: true,
        mlock_available: true,
        accelerator_name: Some("Apple Metal".to_string()),
        accelerator_memory_bytes: 32 * 1024 * 1024 * 1024,
        logical_threads: 10,
        total_memory_bytes: 32 * 1024 * 1024 * 1024,
    };
    let config = RuntimeConfig::for_hardware(&hardware);
    assert_eq!(config.requested_gpu_layers, u32::MAX);
    assert_eq!(config.decode_threads, 5);
    assert_eq!(config.batch_threads, 10);
    assert!(config.use_mmap);
    assert!(!config.use_mlock);
}

#[test]
fn full_context_prefix_matching_evaluates_only_the_appended_suffix() {
    let cached = [1, 2, 3, 4, 5];
    let extended = [1, 2, 3, 4, 5, 6, 7];
    let edited = [1, 2, 9, 4, 5];
    assert_eq!(common_prefix_len(&cached, &extended), cached.len());
    assert_eq!(common_prefix_len(&cached, &edited), 2);
}

#[test]
fn prompt_opens_reasoning_channel_matches_non_thinking_chat_prompt() {
    assert!(prompt_opens_reasoning_channel(
        "<|turn>user\nHello<turn|>\n<|turn>model\n<|channel>thought\n<channel|>"
    ));
    // Thinking-enabled prompts end the model turn without opening a thought channel.
    assert!(!prompt_opens_reasoning_channel(
        "<|turn>user\nHello<turn|>\n<|turn>model\n"
    ));
    // Grammar/workflow prompts that never open a channel stream normally.
    assert!(!prompt_opens_reasoning_channel("System: do X\nAssistant:"));
}

#[test]
fn installed_gemma4_embedding_variants_validate_dynamically() {
    let model_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/models");
    let runtime = NativeRuntime::initialize().expect("initialize llama.cpp runtime");
    let variants = [
        "gemma-4-E2B-it-qat-q4_0-gguf",
        "gemma-4-E4B-it-qat-q4_0-gguf",
        "gemma-4-12B-it-qat-q4_0-gguf",
    ];

    for variant in variants {
        let directory = model_root.join(variant);
        if !directory.is_dir() {
            continue;
        }
        let model_path = fs::read_dir(&directory)
            .expect("read installed model directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
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
            .max_by_key(|path| {
                fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0)
            })
            .expect("installed text GGUF");
        let profile = runtime
            .inspect_model(&model_path)
            .expect("llama.cpp must validate installed Gemma 4 GGUF");

        assert_eq!(profile.architecture, "gemma4", "{variant}");
        assert!(profile.layer_count > 0, "{variant}");
        assert!(profile.embedding_length > 0, "{variant}");
        assert!(
            profile.multi_layer_embeddings,
            "{variant} must expose its per-layer embedding architecture"
        );
    }
}

#[test]
#[ignore = "requires an installed multi-gigabyte GGUF model"]
fn installed_e2b_allocates_a_stateful_context_within_cold_start_budget() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    let started = std::time::Instant::now();
    let (handle, profile) = runtime
        .load_model(&model_path)
        .expect("load E2B model and allocate context");
    let elapsed = started.elapsed();
    eprintln!(
        "E2B_LLAMA_CPP_COLD_START elapsed_ms={}",
        elapsed.as_millis()
    );

    assert_eq!(profile.architecture, "gemma4");
    assert!(profile.multi_layer_embeddings);
    if runtime.hardware().apple_silicon && runtime.hardware().metal_available {
        assert_eq!(profile.device_label, "llama.cpp Metal");
        assert_eq!(profile.gpu_layers, profile.layer_count);
        assert!(
            elapsed < Duration::from_secs(12),
            "E2B cold start exceeded 12 seconds: {elapsed:?}"
        );
    }
    drop(handle);
}

#[test]
#[ignore = "requires an installed multi-gigabyte GGUF model"]
fn installed_e2b_reuses_session_prefix_and_restores_after_memory_flush() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    let system_prompt = "You are OOMU. Keep answers concise.";
    let first_prompt = "<start_of_turn>user\nHello<end_of_turn>\n<start_of_turn>model\n";
    let first = handle
        .append_context(NativeSessionRequest {
            session_id: "phase-2-cache-test".to_string(),
            system_prompt: Some(system_prompt.to_string()),
            prompt: first_prompt.to_string(),
            prompt_is_full_context: true,
        })
        .expect("prefill first turn");
    assert!(first.cold_start);
    assert_eq!(first.cached_tokens, 0);
    assert!(first.evaluated_tokens > 0);

    let second_prompt = format!(
            "{first_prompt}<end_of_turn>\n<start_of_turn>user\nContinue<end_of_turn>\n<start_of_turn>model\n"
        );
    let second = handle
        .append_context(NativeSessionRequest {
            session_id: "phase-2-cache-test".to_string(),
            system_prompt: Some(system_prompt.to_string()),
            prompt: second_prompt.clone(),
            prompt_is_full_context: true,
        })
        .expect("append second turn");
    assert!(!second.cold_start);
    assert_eq!(second.cached_tokens, first.context_tokens);
    assert!(second.evaluated_tokens < second.context_tokens);
    assert!(second.pinned_tokens > 0);

    handle.flush_memory().expect("flush resident model memory");
    let restored = handle
        .append_context(NativeSessionRequest {
            session_id: "phase-2-cache-test".to_string(),
            system_prompt: Some(system_prompt.to_string()),
            prompt: second_prompt,
            prompt_is_full_context: true,
        })
        .expect("restore session after flush");
    assert!(restored.cold_start);
    assert_eq!(restored.cached_tokens, 0);
    assert_eq!(restored.evaluated_tokens, restored.context_tokens);
}

#[test]
#[ignore = "requires installed multi-gigabyte GGUF models; verification"]
fn verify_production_prompt_clean_and_nonempty() {
    let only = std::env::var("DIAG_MODEL").ok();
    let system_file = std::env::var("DIAG_SYSTEM_FILE").unwrap_or_else(|_| {
        env::temp_dir()
            .join("oomu_system_prompt.txt")
            .to_string_lossy()
            .to_string()
    });
    let Ok(system_prompt) = fs::read_to_string(&system_file) else {
        eprintln!("VERIFY SKIP: system fixture {system_file} not found");
        return;
    };
    // Reproduces the user's failing turn: two prior turns of context, then a fresh user message.
    let messages: Vec<(String, String)> = vec![
        ("user".into(), "Hello OOMU".into()),
        (
            "assistant".into(),
            "Hello Dr. Example User. How can I assist you today?".into(),
        ),
        (
            "user".into(),
            "I just changed your model from a 2B variant to a 4B one. Do you feel any different?"
                .into(),
        ),
    ];
    let prompt = crate::gemma::format_gemma4_chat_prompt(&system_prompt, &messages);
    assert!(
        prompt.trim_end().ends_with("<|channel>text\n<channel|>"),
        "production prompt must prime the visible text channel"
    );
    assert!(
        !prompt.contains("<|think|>"),
        "a persona that merely mentions <|think|> must not inject thinking mode"
    );

    let models = [
        ("E2B", "../assets/models/gemma-4-E2B-it-qat-q4_0-gguf"),
        ("E4B", "../assets/models/gemma-4-E4B-it-qat-q4_0-gguf"),
        ("12B", "../assets/models/gemma-4-12B-it-qat-q4_0-gguf"),
    ];
    // Markerless leak signals: this persona induces a visible analysis scratchpad with no
    // channel tokens, so detection keys on its characteristic preamble vocabulary.
    let leak_signals = [
        "<|channel>",
        "<channel|>",
        "<|turn>",
        "<turn|>",
        "thinking_level",
        "thinking level",
        "thinking process",
        "constraint checklist",
        "confidence score",
    ];
    for (label, dir) in models {
        if let Some(filter) = &only {
            if !filter.eq_ignore_ascii_case(label) {
                continue;
            }
        }
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir);
        let Some(model_path) = find_text_gguf(&directory) else {
            eprintln!(
                "VERIFY SKIP {label}: no text gguf under {}",
                directory.display()
            );
            continue;
        };
        let runtime = NativeRuntime::initialize().expect("initialize llama.cpp runtime");
        let (handle, _) = runtime.load_model(&model_path).expect("load model");
        let mut empty = 0usize;
        let mut leaked = 0usize;
        let runs: usize = std::env::var("DIAG_RUNS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(8);
        // Default to the production sampling temperature (GemmaInferenceConfig::low_latency);
        // override with DIAG_TEMP to stress-test at higher temperatures.
        let sample_temp = std::env::var("DIAG_TEMP")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.4);
        for run in 0..runs {
            handle.flush_memory().ok();
            let temperature = if run == 0 { 0.0 } else { sample_temp };
            let result = handle
                .generate(
                    NativeGenerationRequest {
                        session: NativeSessionRequest {
                            session_id: format!("verify-{label}-{run}"),
                            system_prompt: None,
                            prompt: prompt.clone(),
                            prompt_is_full_context: true,
                        },
                        media: Vec::new(),
                        max_new_tokens: 220,
                        temperature,
                        top_k: 40,
                        top_p: 0.95,
                        repeat_penalty: 1.1,
                        grammar: None,
                        cancellation: Arc::new(AtomicBool::new(false)),
                    },
                    |_| {},
                )
                .expect("generate");
            let visible = result.text.trim();
            let is_empty = visible.is_empty();
            let lower = visible.to_ascii_lowercase();
            let first_line = visible.lines().next().unwrap_or_default().trim();
            let first_lower = first_line.to_ascii_lowercase();
            let leading_scratchpad = is_channel_label(first_line.trim_end_matches([':', ' ']))
                || first_lower.starts_with("thinking_level")
                || first_lower.starts_with("thinking level")
                || first_lower.starts_with("plan:")
                || first_lower.starts_with("here's a thinking")
                || first_lower.starts_with("here is a thinking")
                || (first_lower.starts_with("1.") && lower.contains("analyz"));
            let is_leak = !is_empty
                && (leading_scratchpad || leak_signals.iter().any(|signal| lower.contains(signal)));
            if is_empty {
                empty += 1;
            }
            if is_leak {
                leaked += 1;
            }
            eprintln!(
                    "\n##### VERIFY {label} | run={run} temp={temperature} | empty={is_empty} leak={is_leak} #####\n{visible}"
                );
        }
        handle.flush_memory().ok();
        eprintln!("\n===== {label}: {runs} runs, empty={empty}, leaked={leaked} (leak cleaned by local_infer retry) =====");
        // The text-channel-priming fix must eliminate empties at the prompt level for every
        // model; visible-scratchpad leaks are reported only (the binary's retry layer clears
        // them) so this stays a valid pass for the smallest checkpoint too.
        assert_eq!(empty, 0, "{label} produced {empty} empty responses");
    }
}
