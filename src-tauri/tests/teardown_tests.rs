use oomu_lib::gemma::{GemmaService, GemmaStatus};
use std::path::PathBuf;

#[test]
fn test_model_shutdown_does_not_leak_or_abort() {
    let service = GemmaService::new_loading();

    if let Some(model_dir) = std::env::var_os("OOMU_TEARDOWN_TEST_MODEL_DIR") {
        service
            .load_model_from_dir(PathBuf::from(model_dir))
            .expect("configured GGUF model loads before teardown");
    }

    service
        .force_shutdown_native_model()
        .expect("native model shutdown completes without aborting");
    assert!(matches!(service.get_status(), GemmaStatus::Shutdown));

    service
        .force_shutdown_native_model()
        .expect("native model shutdown remains idempotent");
    assert!(matches!(service.get_status(), GemmaStatus::Shutdown));
}
