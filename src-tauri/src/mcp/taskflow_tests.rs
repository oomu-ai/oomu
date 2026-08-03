use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

const TEST_TRUNCATION_NOTICE: &str = "La selección está incompleta.";

fn temp_root(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default()
    ))
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[test]
fn localized_picker_text_is_sanitized_without_an_english_fallback() {
    assert_eq!(
        validate_selection_id("selection-case-123").unwrap(),
        "selection-case-123"
    );
    for invalid in [
        "selection-",
        "selection-A",
        "selection-a_b",
        "selection-a--b",
        "../selection-a",
    ] {
        assert!(validate_selection_id(invalid).is_err());
    }
    assert!(
        validate_selection_id(&format!("selection-{}", "a".repeat(MAX_SELECTION_ID_BYTES)))
            .is_err()
    );
    assert_eq!(
        sanitize_dialog_title("  Choisir\n\0le dossier  ").unwrap(),
        "Choisir le dossier"
    );
    assert_eq!(
        sanitize_truncation_notice("  Selección parcial.\r\nRevise la nota.\0  ").unwrap(),
        "Selección parcial.\nRevise la nota."
    );
    assert!(sanitize_dialog_title(" \0\n ").is_err());
    assert!(sanitize_truncation_notice(&"é".repeat(MAX_TRUNCATION_NOTICE_BYTES)).is_err());
}

#[test]
fn workflow_source_staging_preserves_nested_readable_text_files() {
    let root = temp_root("oomu-workflow-source-nested");
    let source = root.join("source");
    let sandbox = root.join("sandbox");
    fs::create_dir_all(source.join("notes/deep")).expect("source tree creates");
    fs::write(source.join("brief.txt"), "alpha notes").expect("brief writes");
    fs::write(source.join("notes/deep/context.md"), "beta details").expect("context writes");

    let metadata = stage_workflow_source_folder(
        &source,
        &sandbox,
        "selection-nested",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect("source stages");

    assert_eq!(metadata.folder_name, "source");
    assert_eq!(
        metadata.folder_path,
        "workspace/selections/selection-nested"
    );
    assert_eq!(metadata.file_count, 2);
    assert_eq!(
        metadata.total_bytes,
        "alpha notes".len() + "beta details".len()
    );
    assert!(!metadata.truncated);
    assert_eq!(
        fs::read_to_string(sandbox.join("workspace/selections/selection-nested/brief.txt"))
            .unwrap(),
        "alpha notes"
    );
    assert_eq!(
        fs::read_to_string(
            sandbox.join("workspace/selections/selection-nested/notes/deep/context.md")
        )
        .unwrap(),
        "beta details"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_source_selections_remain_isolated_by_identifier() {
    let root = temp_root("oomu-workflow-source-isolated");
    let source_a = root.join("source-a");
    let source_b = root.join("source-b");
    let sandbox = root.join("sandbox");
    fs::create_dir_all(&source_a).expect("source a creates");
    fs::create_dir_all(&source_b).expect("source b creates");
    fs::write(source_a.join("a.txt"), "alpha selection").expect("source a writes");
    fs::write(source_b.join("b.txt"), "beta selection").expect("source b writes");

    let first = stage_workflow_source_folder(
        &source_a,
        &sandbox,
        "selection-alpha",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect("first source stages");
    let second = stage_workflow_source_folder(
        &source_b,
        &sandbox,
        "selection-beta",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect("second source stages");

    assert_eq!(first.folder_path, "workspace/selections/selection-alpha");
    assert_eq!(second.folder_path, "workspace/selections/selection-beta");
    assert_eq!(
        fs::read_to_string(sandbox.join("workspace/selections/selection-alpha/a.txt")).unwrap(),
        "alpha selection"
    );
    assert!(!sandbox
        .join("workspace/selections/selection-alpha/b.txt")
        .exists());
    assert_eq!(
        fs::read_to_string(sandbox.join("workspace/selections/selection-beta/b.txt")).unwrap(),
        "beta selection"
    );
    assert!(!sandbox
        .join("workspace/selections/selection-beta/a.txt")
        .exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_source_staging_ignores_hidden_binary_and_symlink_entries() {
    let root = temp_root("oomu-workflow-source-filtered");
    let source = root.join("source");
    let sandbox = root.join("sandbox");
    fs::create_dir_all(source.join(".private")).expect("hidden tree creates");
    fs::write(source.join("visible.md"), "visible notes").expect("visible writes");
    fs::write(source.join(".hidden.txt"), "hidden notes").expect("hidden writes");
    fs::write(source.join(".private/secret.txt"), "secret notes").expect("secret writes");
    fs::write(source.join("binary.dat"), [0xff, 0xfe, 0xfd]).expect("binary writes");
    fs::write(source.join("nul.dat"), b"text\0payload").expect("nul writes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = root.join("outside.txt");
        fs::write(&outside, "outside notes").expect("outside writes");
        symlink(&outside, source.join("linked.txt")).expect("file symlink creates");
        symlink(&root, source.join("linked-folder")).expect("folder symlink creates");
    }

    let metadata = stage_workflow_source_folder(
        &source,
        &sandbox,
        "selection-filtered",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect("source stages");
    let staged = sandbox.join("workspace/selections/selection-filtered");

    assert_eq!(metadata.file_count, 1);
    assert_eq!(
        fs::read_to_string(staged.join("visible.md")).unwrap(),
        "visible notes"
    );
    assert!(!staged.join(".hidden.txt").exists());
    assert!(!staged.join(".private").exists());
    assert!(!staged.join("binary.dat").exists());
    assert!(!staged.join("nul.dat").exists());
    assert!(!staged.join("linked.txt").exists());
    assert!(!staged.join("linked-folder").exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn truncated_staging_prioritizes_a_localized_note_without_overwriting_source() {
    let root = temp_root("oomu-workflow-source-note");
    let source = root.join("source");
    let sandbox = root.join("sandbox");
    let source_note = source.join(SELECTION_NOTE_FILE_NAME);
    fs::create_dir_all(&source).expect("source creates");
    fs::write(&source_note, "user-authored source note").expect("source note writes");
    fs::write(source.join("a.txt"), "alpha notes").expect("a writes");
    fs::write(source.join("b.txt"), "beta details").expect("b writes");

    let metadata = stage_workflow_source_folder(
        &source,
        &sandbox,
        "selection-note",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect("source stages with a completeness note");
    let staged_note = sandbox
        .join("workspace/selections/selection-note")
        .join(SELECTION_NOTE_FILE_NAME);

    assert!(metadata.truncated);
    assert_eq!(metadata.file_count, 3);
    assert_eq!(
        fs::read_to_string(&staged_note).unwrap(),
        TEST_TRUNCATION_NOTICE
    );
    assert_eq!(
        fs::read_to_string(&source_note).unwrap(),
        "user-authored source note"
    );

    let server = NativeTaskflowServer::new(sandbox.clone()).expect("server initializes");
    let read = server
        .folder_read(&args(json!({
            "folderPath": "workspace/selections/selection-note",
            "maxFiles": 1
        })))
        .expect("folder read succeeds");
    assert_eq!(read["structuredContent"]["fileCount"], json!(1));
    assert!(read["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains(TEST_TRUNCATION_NOTICE));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_source_staging_honors_bounds_and_replaces_only_after_preparation() {
    let root = temp_root("oomu-workflow-source-replace");
    let source = root.join("source");
    let invalid_source = root.join("invalid-source");
    let sandbox = root.join("sandbox");
    let old_input = sandbox.join("workspace/selections/selection-replace");
    fs::create_dir_all(&old_input).expect("old input creates");
    fs::write(old_input.join("old.txt"), "keep until ready").expect("old input writes");
    fs::create_dir_all(&source).expect("source creates");
    fs::write(source.join("a.txt"), "aaaa").expect("a writes");
    fs::write(source.join("b.txt"), "bbbbb").expect("b writes");
    fs::write(source.join("c.txt"), "cccc").expect("c writes");
    fs::write(source.join("d.txt"), "dddd").expect("d writes");
    fs::write(source.join("e.txt"), "eeee").expect("e writes");
    let limits = StagingLimits {
        max_files: 2,
        max_file_bytes: 4,
        max_total_bytes: 8,
        ..DEFAULT_STAGING_LIMITS
    };

    let metadata =
        stage_workflow_source_folder(&source, &sandbox, "selection-replace", "cut", limits)
            .expect("bounded source stages");

    assert_eq!(metadata.file_count, 2);
    assert_eq!(metadata.total_bytes, 7);
    assert!(metadata.truncated);
    assert_eq!(fs::read_to_string(old_input.join("a.txt")).unwrap(), "aaaa");
    assert_eq!(
        fs::read_to_string(old_input.join(SELECTION_NOTE_FILE_NAME)).unwrap(),
        "cut"
    );
    assert!(!old_input.join("b.txt").exists());
    assert!(!old_input.join("c.txt").exists());
    assert!(!old_input.join("d.txt").exists());
    assert!(!old_input.join("e.txt").exists());
    assert!(!old_input.join("old.txt").exists());

    fs::create_dir_all(&invalid_source).expect("invalid source creates");
    fs::write(invalid_source.join("binary.dat"), [0xff, 0xfe]).expect("invalid source writes");
    let error = stage_workflow_source_folder(
        &invalid_source,
        &sandbox,
        "selection-replace",
        "cut",
        limits,
    )
    .expect_err("an invalid replacement is rejected");
    assert!(error.contains("no readable UTF-8 text files"));
    assert_eq!(fs::read_to_string(old_input.join("a.txt")).unwrap(), "aaaa");
    assert_eq!(
        fs::read_to_string(old_input.join(SELECTION_NOTE_FILE_NAME)).unwrap(),
        "cut"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn workflow_source_staging_rejects_a_folder_without_readable_input() {
    let root = temp_root("oomu-workflow-source-empty");
    let source = root.join("source");
    let sandbox = root.join("sandbox");
    fs::create_dir_all(&source).expect("source creates");
    fs::write(source.join("binary.dat"), [0xff, 0xfe]).expect("binary writes");
    fs::write(source.join(".hidden.txt"), "hidden").expect("hidden writes");

    let error = stage_workflow_source_folder(
        &source,
        &sandbox,
        "selection-empty",
        TEST_TRUNCATION_NOTICE,
        DEFAULT_STAGING_LIMITS,
    )
    .expect_err("folder without readable input is rejected");

    assert!(error.contains("no readable UTF-8 text files"));
    assert!(!sandbox
        .join("workspace/selections/selection-empty")
        .exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn taskflow_reads_folder_writes_and_previews_a_report() {
    let root = temp_root("oomu-taskflow");
    let server = NativeTaskflowServer::new(root.clone()).expect("server initializes");

    let input = root.join("workspace/selections/selection-taskflow-test");
    fs::create_dir_all(input.join("nested")).expect("input dir creates");
    fs::write(input.join("a.txt"), "alpha notes").expect("a writes");
    fs::write(input.join("nested/b.md"), "beta details").expect("b writes");

    let read = server
        .folder_read(&args(json!({
            "folderPath": "workspace/selections/selection-taskflow-test",
            "maxFiles": 24
        })))
        .expect("folder_read succeeds");
    assert_eq!(read["isError"], json!(false));
    assert_eq!(read["structuredContent"]["fileCount"], json!(2));
    let scanned = read["content"][0]["text"].as_str().unwrap();
    assert!(scanned.contains("alpha notes"));
    assert!(scanned.contains("beta details"));

    let write = server
        .write_markdown_report(&args(json!({
            "reportPath": "workspace/report.md",
            "content": "# Summary\nGrounded in the scanned folder."
        })))
        .expect("write succeeds");
    assert_eq!(write["isError"], json!(false));
    assert_eq!(
        write["structuredContent"]["relativePath"],
        json!("workspace/report.md")
    );
    assert!(root.join("workspace/report.md").is_file());

    let preview = server
        .preview_report(&args(json!({ "reportPath": "workspace/report.md" })))
        .expect("preview succeeds");
    assert_eq!(preview["isError"], json!(false));
    assert!(preview["structuredContent"]["content"]
        .as_str()
        .unwrap()
        .contains("Grounded in the scanned folder."));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_collection_reports_probe_failure_instead_of_empty_success() {
    let missing = temp_root("oomu-taskflow-missing").join("not-created");
    let mut files = Vec::new();
    let error = collect_text_files(&missing, 0, &mut files)
        .expect_err("an unreadable folder must not become an observed empty folder");
    assert!(error.contains("could not inspect"));
    assert!(files.is_empty());
}

#[test]
fn taskflow_requires_report_content() {
    let root = temp_root("oomu-taskflow-empty");
    let server = NativeTaskflowServer::new(root.clone()).expect("server initializes");

    let error = server
        .write_markdown_report(&args(json!({ "reportPath": "report.md" })))
        .expect_err("missing content is rejected");
    assert!(error.contains("requires report content"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn folder_read_requires_an_explicit_path_and_creates_no_fixture_files() {
    let root = temp_root("oomu-taskflow-no-fixture");
    let server = NativeTaskflowServer::new(root.clone()).expect("server initializes");

    let error = server
        .folder_read(&args(json!({})))
        .expect_err("missing folder path is rejected");
    assert!(error.contains("explicit approved sandbox folder path"));
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn taskflow_rejects_paths_outside_the_sandbox() {
    let root = temp_root("oomu-taskflow-escape");
    let server = NativeTaskflowServer::new(root.clone()).expect("server initializes");

    let escaped = server
        .write_markdown_report(&args(json!({
            "reportPath": "/private/etc/oomu-escape.md",
            "content": "nope"
        })))
        .expect_err("outside path is rejected");
    assert_eq!(escaped, "Path escapes the local sandbox.");

    let read_escape = server
        .folder_read(&args(json!({ "folderPath": "/private/etc" })))
        .expect_err("outside folder is rejected");
    assert_eq!(read_escape, "Path escapes the local sandbox.");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn taskflow_handles_requests_through_the_jsonrpc_surface() {
    let root = temp_root("oomu-taskflow-rpc");
    let server = NativeTaskflowServer::new(root.clone()).expect("server initializes");

    let list = server.handle_request(JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "tools/list".to_string(),
        params: json!({}),
        id: json!(1),
    });
    let tools = list.result.expect("tools/list returns result")["tools"]
        .as_array()
        .expect("tools is an array")
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str).map(str::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        tools,
        vec!["folder_read", "write_markdown_report", "preview_report"]
    );

    let _ = fs::remove_dir_all(root);
}
