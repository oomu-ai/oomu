use std::{fs, path::PathBuf};

fn manifest_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn unstable_child_webview_routes_file_drops_through_webview_events() {
    let root = manifest_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    let app = fs::read_to_string(root.join("src/lib.rs")).expect("read app entrypoint");

    assert!(
        cargo.contains("\"unstable\""),
        "this contract applies while Tauri's child-webview runtime is enabled",
    );
    assert!(
        app.contains(".on_webview_event(|webview, event|")
            && app.contains("emit_local_context_drag(&webview.window(), drag_event)"),
        "child-webview drops must be handled on Tauri's webview event channel",
    );
}
