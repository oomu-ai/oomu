use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

const LLAMA_CPP_FIXTURE_VERSION: &str = "llama-cpp-2-0.1.151";
const FIXTURE_RELATIVE_PATH: &str = "src/gguf/ggml-vocab-bert-bge.gguf";
const FIXTURE_SHA256: &str = "fbcbe22278fb302694d5f4a41bfe48c5f90e8e3554eab1c0435387dff654a854";
const CANONICAL_MODELS: [(&str, &str); 3] = [
    ("gemma-4-E2B-it-qat-q4_0-gguf", "google/gemma-4-E2B-it"),
    ("gemma-4-E4B-it-qat-q4_0-gguf", "google/gemma-4-E4B-it"),
    ("gemma-4-12B-it-qat-q4_0-gguf", "google/gemma-4-12B-it"),
];

pub(crate) fn root() -> PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(create_fixture_root).clone()
}

fn create_fixture_root() -> PathBuf {
    let source = fixture_source();
    let bytes = fs::read(&source).expect("locked llama-cpp GGUF fixture is readable");
    assert_eq!(
        crate::foundation::digest::sha256_hex(&bytes),
        FIXTURE_SHA256,
        "locked llama-cpp GGUF fixture digest changed"
    );
    let root = env::temp_dir().join(format!(
        "oomu-valid-local-model-fixtures-{}-{}",
        std::process::id(),
        crate::foundation::clock::unix_time_ns_u128()
    ));
    for (canonical_id, configured_name) in CANONICAL_MODELS {
        let directory = root.join(canonical_id);
        fs::create_dir_all(&directory).expect("test local-model directory is created");
        fs::write(directory.join("model.gguf"), &bytes)
            .expect("parser-valid GGUF test fixture is written");
        fs::write(
            directory.join("config.json"),
            serde_json::json!({"_name_or_path": configured_name}).to_string(),
        )
        .expect("test local-model identity metadata is written");
    }
    root
}

fn fixture_source() -> PathBuf {
    cargo_registry_roots()
        .into_iter()
        .flat_map(|registry| fs::read_dir(registry).ok().into_iter().flatten())
        .filter_map(Result::ok)
        .map(|entry| {
            entry
                .path()
                .join(LLAMA_CPP_FIXTURE_VERSION)
                .join(FIXTURE_RELATIVE_PATH)
        })
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!(
                "locked llama-cpp GGUF fixture {LLAMA_CPP_FIXTURE_VERSION}/{FIXTURE_RELATIVE_PATH} was not found"
            )
        })
}

fn cargo_registry_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(cargo_home) = env::var_os("CARGO_HOME") {
        roots.push(Path::new(&cargo_home).join("registry/src"));
    }
    if let Some(home) = env::var_os("HOME") {
        let default = Path::new(&home).join(".cargo/registry/src");
        if !roots.contains(&default) {
            roots.push(default);
        }
    }
    roots
}
