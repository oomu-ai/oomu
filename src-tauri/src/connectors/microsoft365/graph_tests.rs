use super::*;

#[test]
fn only_remote_mutations_are_effectful() {
    assert!(policy(OUTLOOK_MAIL_DRAFT).unwrap().effectful);
    assert!(policy(ONEDRIVE_WRITE).unwrap().effectful);
    assert!(policy(SHAREPOINT_WRITE).unwrap().effectful);
    assert!(!policy(OUTLOOK_CALENDAR_DRAFT).unwrap().remote);
    assert!(!policy(TEAMS_DRAFT).unwrap().effectful);
    assert!(!policy(TEAMS_LIST).unwrap().effectful);
    assert!(!policy(SHAREPOINT_RESOLVE).unwrap().effectful);
}

#[test]
fn discovery_capabilities_share_the_existing_least_privilege_grants() {
    assert_eq!(operation_for_capability("list_chats").unwrap(), TEAMS_LIST);
    assert_eq!(
        operation_for_capability("find_team_site").unwrap(),
        SHAREPOINT_RESOLVE
    );
    let adapter = &MICROSOFT_ADAPTER;
    assert!(adapter
        .capabilities_for_operation(TEAMS_SEARCH)
        .contains(&"list_chats"));
    assert!(adapter
        .capabilities_for_operation(SHAREPOINT_SEARCH)
        .contains(&"find_team_site"));
}

#[test]
fn effectful_approval_arguments_are_exact_validated_and_secret_free() {
    let mail = MICROSOFT_ADAPTER
        .approval_arguments(
            OUTLOOK_MAIL_DRAFT,
            &json!({
                "to":["person@example.com"],
                "cc":"reviewer@example.com",
                "subject":"Quarterly review",
                "body":"Please review the attached summary.",
                "accessToken":"credential-canary",
                "unknown":"must-not-cross-approval-boundary"
            }),
        )
        .unwrap();
    assert_eq!(mail["to"], json!(["person@example.com"]));
    assert_eq!(mail["cc"], json!(["reviewer@example.com"]));
    assert_eq!(mail["subject"], "Quarterly review");
    assert_eq!(mail["body"], "Please review the attached summary.");
    assert!(mail.get("accessToken").is_none());
    assert!(mail.get("unknown").is_none());

    let file = MICROSOFT_ADAPTER
        .approval_arguments(
            ONEDRIVE_WRITE,
            &json!({
                "path":"reports/q3.txt",
                "content":"raw-file-content-canary",
                "contentType":"text/plain",
                "replaceExisting":true,
                "expectedETag":"etag-1",
                "credential":"credential-canary"
            }),
        )
        .unwrap();
    assert_eq!(file["path"], "reports/q3.txt");
    assert_eq!(file["contentBytes"], 23);
    assert_eq!(file["replaceExisting"], true);
    assert_eq!(file["expectedETag"], "etag-1");
    assert!(file.get("content").is_none());
    assert!(file.get("credential").is_none());
    assert_ne!(file["contentSha256"], "raw-file-content-canary");
    assert!(MICROSOFT_ADAPTER
        .approval_arguments(
            ONEDRIVE_WRITE,
            &json!({"path":"a.txt","content":"x","replaceExisting":true})
        )
        .is_err());
}

#[test]
fn paging_tokens_are_removed_and_reported_as_partial() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/microsoft_365/graph_partial_page.json"
    ))
    .unwrap();
    let (clean, partial) = strip_paging_token(fixture);
    assert!(partial);
    assert!(clean.get("@odata.nextLink").is_none());
    let mut cited = clean;
    attach_source_citations(OUTLOOK_MAIL_SEARCH, &json!({"query":"secret"}), &mut cited).unwrap();
    let citations = cited["sourceCitations"].as_array().unwrap();
    assert_eq!(citations.len(), 1);
    assert!(citations[0]
        .as_str()
        .unwrap()
        .starts_with("graph://outlook/mail/message/"));
    assert!(!citations[0].as_str().unwrap().contains("secret"));
}

#[test]
fn teams_local_filter_reports_truncated_matching_fixture_as_partial() {
    let mut fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/microsoft_365/teams_search_26_matches.json"
    ))
    .unwrap();
    let partial = filter_chat_matches(&mut fixture, "quarterly", false).unwrap();
    assert!(partial);
    assert_eq!(fixture["value"].as_array().unwrap().len(), 25);
}

#[test]
fn file_writes_require_safe_paths_and_etag_policy() {
    assert!(safe_relative_path("reports/q3.txt").is_ok());
    assert!(safe_relative_path("../secret.txt").is_err());
    assert!(safe_relative_path("/").is_err());
    assert!(safe_relative_path("/reports/q3.txt").is_err());
    assert!(safe_relative_path("reports//q3.txt").is_err());
    let observed = observed_write_result(
        &json!({"id":"item-1","eTag":"etag-2","size":12,"name":"q3.txt"}),
        true,
        "q3.txt",
        12,
    )
    .unwrap();
    assert_eq!(
        observed["mutationPostcondition"],
        "file_replaced_at_expected_etag"
    );
    assert!(observed_write_result(&json!({"id":"item-1"}), false, "q3.txt", 12).is_err());
    assert!(observed_write_result(
        &json!({"id":"item-1","eTag":"etag","size":11,"name":"q3.txt"}),
        false,
        "q3.txt",
        12,
    )
    .is_err());
    assert!(observed_write_result(
        &json!({"id":"item-1","eTag":"etag","size":12,"name":"other.txt"}),
        false,
        "q3.txt",
        12,
    )
    .is_err());
    assert!(observed_write_result(
        &json!({"id":"item-1","eTag":"","size":12,"name":"q3.txt"}),
        false,
        "q3.txt",
        12,
    )
    .is_err());
    let binary = STANDARD.encode([0_u8, 159, 146, 150]);
    let (decoded, content_type) = write_payload(&json!({
        "contentBase64":binary,
        "contentType":"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    }))
    .unwrap();
    assert_eq!(decoded, vec![0_u8, 159, 146, 150]);
    assert!(content_type.contains("spreadsheetml"));
    assert!(write_payload(&json!({"content":"one","contentBase64":"dHdv"})).is_err());
    assert!(write_payload(&json!({"content":"one","contentType":"invalid"})).is_err());
}

#[test]
fn local_drafts_have_observed_non_delivery_postconditions() {
    let calendar=local_draft(OUTLOOK_CALENDAR_DRAFT,&json!({"subject":"Review","startDateTime":"2026-07-12T10:00:00Z","endDateTime":"2026-07-12T10:30:00Z"})).unwrap();
    assert_eq!(calendar.result["eventCreated"], false);
    assert_eq!(calendar.result["invitationsSent"], false);
    let teams = local_draft(TEAMS_DRAFT, &json!({"chatId":"chat","text":"Hello"})).unwrap();
    assert_eq!(teams.result["posted"], false);
    assert!(observed_draft_result(&json!({"id":"","isDraft":true})).is_err());
    assert!(observed_draft_result(&json!({"id":"draft-1","isDraft":false})).is_err());
    assert!(observed_draft_result(&json!({"id":"draft-1","isDraft":true})).is_ok());
    assert!(local_draft(
        OUTLOOK_CALENDAR_DRAFT,
        &json!({
            "subject":"Review",
            "startDateTime":"2026-07-12T10:00:00Z",
            "endDateTime":"2026-07-12T10:30:00Z",
            "timeZone":42
        })
    )
    .is_err());
    assert!(recipients(&json!({"to":["person@example.com",42]}), "to").is_err());
}

#[test]
fn citations_bind_exact_objects_without_queries_or_tokens() {
    let mail = object_citation(
        OUTLOOK_MAIL_READ,
        &json!({"messageId":"AAMk/message?id=secret"}),
        &Value::Null,
        "fallback",
    );
    assert!(mail.starts_with("graph://outlook/mail/message/"));
    assert!(!mail.contains("?"));
    assert!(!mail.contains("secret"));
    let sharepoint = object_citation(
        SHAREPOINT_WRITE,
        &json!({"siteId":"site-1","path":"reports/q3.txt"}),
        &json!({"id":"item-7"}),
        "fallback",
    );
    assert!(sharepoint.contains("/site/") && sharepoint.contains("/item/"));
    assert!(!sharepoint.contains("reports"));
}
