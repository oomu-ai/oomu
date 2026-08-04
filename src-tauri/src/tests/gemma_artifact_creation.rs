use super::*;

#[test]
fn common_artifact_names_create_hello_world_in_the_requested_user_folder() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME");
    for (label, format, folder) in [
        ("PDF document", "pdf", "Downloads"),
        ("Word doc", "docx", "Documents"),
        ("PowerPoint", "pptx", "Desktop"),
        ("Excel file", "xlsx", "Downloads"),
    ] {
        let objective =
            format!("Create a {label} in my {folder} folder with content “Hello World”.");
        let draft = generated_plan_from_text(
            objective,
            "model emitted malformed action-plan text".to_string(),
        );
        let expected = home.join(folder).join(format!("hello_world.{format}"));
        assert!(
            matches!(
                &draft.steps[0].tool,
                GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
                    if operation == "create_file"
                        && arguments["file"]["format"] == format
                        && arguments["file"]["destinationPath"] == expected.to_string_lossy().as_ref()
                        && arguments["file"]["title"] == "hello_world"
                        && arguments["file"]["content"] == "Hello World"
            ),
            "{label}"
        );
    }
}

#[test]
fn common_artifact_without_a_destination_uses_downloads_and_an_obvious_name() {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .expect("HOME");
    let draft = generated_plan_from_text(
        "Create a Word doc with “Hello World”.".to_string(),
        "model emitted malformed action-plan text".to_string(),
    );
    let expected = home.join("Downloads/hello_world.docx");
    assert!(matches!(
        &draft.steps[0].tool,
        GeneratedToolDraft::RegisteredTaskTool { operation, arguments }
            if operation == "create_file"
                && arguments["file"]["format"] == "docx"
                && arguments["file"]["destinationPath"] == expected.to_string_lossy().as_ref()
                && arguments["file"]["content"] == "Hello World"
    ));
}
