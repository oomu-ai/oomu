use super::*;

pub(crate) fn register_test_tools() {
    let _ = crate::artifacts::register_file_task_tool();
    let _ = crate::tools::official_page::register_task_tool();
    let _ = crate::tools::project_file::register_task_tool();
    let _ = crate::tools::milestone_analysis::register_task_tool();
    let _ = crate::tools::supplier_exception::register_task_tool();
    let _ = crate::tools::evidence_report_composition::register_task_tool();
    let _ = crate::tools::evidence_report_validation::register_task_tool();
    let _ = crate::tools::system_calendar_event::register_task_tool();
    let _ = crate::tools::system_mail_send::register_task_tool();
}

#[test]
fn registered_output_schemas_describe_domain_data_not_transport_metadata() {
    let create_file = output_schema("create_file");
    assert!(create_file.pointer("/properties/verified").is_none());
    assert!(create_file
        .pointer("/properties/structuredContent/properties/path")
        .is_some());

    let official_page = output_schema("fetch_official_page");
    for field in [
        "requestedUrl",
        "selectedUrl",
        "attemptedUrls",
        "fallbackUsed",
        "finalUrl",
        "accessedAtUtc",
        "statusCode",
        "contentType",
        "content",
        "contentSha256",
        "contentBytes",
        "contentTruncated",
    ] {
        assert!(
            official_page
                .pointer(&format!("/properties/{field}"))
                .is_some(),
            "missing production receipt field {field}"
        );
        assert!(
            official_page["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|value| value == field)),
            "production receipt field {field} is not required"
        );
    }

    let composer = output_schema("compose_evidence_report");
    assert_eq!(
        composer["properties"]["compositionMethod"],
        json!({"type":"string"})
    );
    assert!(composer["required"]
        .as_array()
        .is_some_and(|required| required.contains(&json!("content"))));
}
