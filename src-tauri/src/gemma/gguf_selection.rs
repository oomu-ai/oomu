use super::{is_gguf_file, sum_local_weight_bytes, GemmaError};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn select_primary_gguf(model_dir: &Path) -> Result<Option<PathBuf>, GemmaError> {
    let preferred = model_dir.join("model.gguf");
    if preferred.is_file() {
        return Ok(Some(preferred));
    }

    let mut candidates = fs::read_dir(model_dir)
        .map_err(|error| GemmaError::io("local model directory read", error))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| is_primary_weight_candidate(path))
        .collect::<Vec<_>>();
    candidates.sort();
    if candidates.is_empty() {
        return Ok(None);
    }

    let directory_name = model_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let canonical_stem = directory_name
        .strip_suffix("-gguf")
        .unwrap_or(&directory_name);
    let canonical = candidates
        .iter()
        .filter(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(&canonical_stem))
        })
        .cloned()
        .collect::<Vec<_>>();

    match (canonical.as_slice(), candidates.as_slice()) {
        ([path], _) => Ok(Some(path.clone())),
        ([], [path]) => Ok(Some(path.clone())),
        _ => Err(ambiguous_primary_error(model_dir, &candidates)),
    }
}

pub(super) fn selected_weight_bytes(
    asset_files: &[PathBuf],
    primary_gguf: Option<&Path>,
) -> Result<u64, GemmaError> {
    match primary_gguf {
        Some(path) => fs::metadata(path)
            .map(|metadata| metadata.len())
            .map_err(|error| GemmaError::io("local model weights metadata", error)),
        None => sum_local_weight_bytes(asset_files),
    }
}

fn is_primary_weight_candidate(path: &Path) -> bool {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    path.is_file()
        && is_gguf_file(path)
        && !filename.contains("mmproj")
        && !filename.contains("draft")
        && !filename.contains("mtp")
        && !filename.ends_with(".part")
}

fn ambiguous_primary_error(model_dir: &Path, candidates: &[PathBuf]) -> GemmaError {
    let names = candidates
        .iter()
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy())
        .collect::<Vec<_>>()
        .join(", ");
    GemmaError {
        code: "local_model_primary_gguf_ambiguous",
        message: format!(
            "The local model directory {} contains multiple non-canonical GGUF weights ({names}). Rename the intended primary weight to model.gguf or to the model directory basename with the trailing -gguf removed.",
            model_dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDirectory {
        path: PathBuf,
        cleanup_root: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let cleanup_root = std::env::temp_dir()
                .join(format!("oomu-primary-gguf-{}-{nonce}", std::process::id()));
            let path = cleanup_root.join(name);
            fs::create_dir_all(&path).expect("create model fixture");
            Self { path, cleanup_root }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.cleanup_root);
        }
    }

    #[test]
    fn canonical_q4_beats_a_larger_noncanonical_q6_weight() {
        let directory = TestDirectory::new("gemma-4-12B-it-qat-q4_0-gguf");
        let canonical = directory.path.join("gemma-4-12b-it-qat-q4_0.gguf");
        let larger_q6 = directory.path.join("gemma4-coding-Q6_K.gguf");
        fs::write(&canonical, [1_u8; 4]).expect("write canonical weight");
        fs::write(&larger_q6, [2_u8; 16]).expect("write larger Q6 weight");

        let selected = select_primary_gguf(&directory.path)
            .expect("selection succeeds")
            .expect("primary GGUF exists");
        assert_eq!(selected, canonical);
        assert_eq!(
            selected_weight_bytes(&[selected.clone(), larger_q6], Some(&selected))
                .expect("read selected byte count"),
            4
        );
    }

    #[test]
    fn multiple_noncanonical_weights_fail_closed_as_ambiguous() {
        let directory = TestDirectory::new("ambiguous-model");
        fs::write(directory.path.join("first.gguf"), [1_u8; 4]).expect("write first weight");
        fs::write(directory.path.join("second.gguf"), [2_u8; 8]).expect("write second weight");

        let error = select_primary_gguf(&directory.path).expect_err("ambiguity must fail closed");
        assert_eq!(error.code, "local_model_primary_gguf_ambiguous");
        assert!(error.message.contains("first.gguf"));
        assert!(error.message.contains("second.gguf"));
    }
}
