use super::*;

fn find_text_gguf(directory: &Path) -> Option<PathBuf> {
    fs::read_dir(directory)
        .ok()?
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
}

// Empirical probe: drive each installed model with a realistic multi-turn history and three
// candidate generation-prompt endings (bare / thought-primed / text-primed), printing the RAW
// token stream and the sanitized output. Set DIAG_MODEL=E2B|E4B|12B to run a single model.
//   cargo test -p oomu_lib --bin '' 2>/dev/null  # (run via lib test target)
//   DIAG_MODEL=E4B cargo test diagnose_prompt_endings_across_models -- --ignored --nocapture

// Prompt-level check of the real production path: build the prompt with the actual
// `format_gemma4_chat_prompt` and a reasoning-heavy persona (point DIAG_SYSTEM_FILE at one, e.g.
// an exported persona fixture; defaults to a temp-dir fixture path and skips if absent), generate
// many times per model, and assert every turn is non-empty. Residual visible scratchpad is only
// REPORTED here: at this single-shot generate layer the smallest checkpoint still leaks ~25%;
// that residue is cleaned end-to-end by the local_infer retry/regeneration safety net.
//   DIAG_MODEL=E4B cargo test verify_production_prompt_clean_and_nonempty -- --ignored --nocapture

mod execution;
mod gguf;
mod lifecycle;
