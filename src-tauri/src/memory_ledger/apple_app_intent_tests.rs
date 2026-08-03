use super::is_explicit_external_apple_app_mutation;

#[test]
fn ordinary_note_language_does_not_target_apple_notes() {
    for request in [
        "Write a Markdown file with a short note confirming contextual file creation.",
        "Create a note in the report about the release date.",
        "Save these notes as a Markdown document.",
    ] {
        assert!(
            !is_explicit_external_apple_app_mutation(request),
            "{request}"
        );
    }
}

#[test]
fn explicit_apple_notes_destination_remains_protected() {
    assert!(is_explicit_external_apple_app_mutation(
        "Write a short note in the Apple Notes app."
    ));
}
