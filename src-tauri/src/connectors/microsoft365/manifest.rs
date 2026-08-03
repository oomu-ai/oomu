use super::super::{ConnectorManifest, ConnectorTool};
use super::contract::*;
use serde_json::{json, Value};

fn list_output(max_items: usize) -> Value {
    json!({
        "type":"object",
        "properties":{
            "value":{"type":"array","maxItems":max_items},
            "sourceCitations":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":2048}}
        },
        "required":["value","sourceCitations"]
    })
}

fn write_input(require_site: bool) -> Value {
    let mut schema = json!({
        "type":"object",
        "additionalProperties":false,
        "properties":{
            "path":{"type":"string","minLength":1,"maxLength":2048},
            "content":{"type":"string","minLength":1,"maxLength":4194304},
            "contentBase64":{"type":"string","minLength":1,"maxLength":5592416,"contentEncoding":"base64"},
            "contentType":{"type":"string","minLength":3,"maxLength":255},
            "replaceExisting":{"type":"boolean","default":false},
            "expectedETag":{"type":"string","minLength":1,"maxLength":256}
        },
        "required":["path"],
        "oneOf":[{"required":["content"]},{"required":["contentBase64"]}],
        "allOf":[{
            "if":{"properties":{"replaceExisting":{"const":true}},"required":["replaceExisting"]},
            "then":{"required":["expectedETag"]}
        }]
    });
    if require_site {
        schema["properties"]["siteId"] = json!({"type":"string","minLength":1,"maxLength":512});
        schema["required"] = json!(["siteId", "path"]);
    }
    schema
}

fn write_output() -> Value {
    json!({
        "type":"object",
        "properties":{
            "id":{"type":"string","minLength":1,"maxLength":512},
            "name":{"type":"string","minLength":1,"maxLength":255},
            "size":{"type":"integer","minimum":1,"maximum":4194304},
            "eTag":{"type":"string","minLength":1,"maxLength":512},
            "webUrl":{"type":["string","null"],"maxLength":4096},
            "mutationPostcondition":{"type":"string","enum":["new_file_created","file_replaced_at_expected_etag"]}
        },
        "required":["id","name","size","eTag","mutationPostcondition"]
    })
}

fn schemas(name: &str) -> (Value, Value) {
    match name {
        OUTLOOK_MAIL_SEARCH => (
            json!({"type":"object","additionalProperties":false,"properties":{"query":{"type":"string","minLength":1,"maxLength":500}},"required":["query"]}),
            list_output(25),
        ),
        OUTLOOK_MAIL_READ => (
            json!({"type":"object","additionalProperties":false,"properties":{"messageId":{"type":"string","minLength":1,"maxLength":512}},"required":["messageId"]}),
            json!({"type":"object","properties":{"id":{"type":"string","minLength":1,"maxLength":512}},"required":["id"]}),
        ),
        OUTLOOK_MAIL_DRAFT => (
            json!({
                "type":"object","additionalProperties":false,
                "properties":{
                    "to":{"oneOf":[{"type":"string","minLength":3,"maxLength":320},{"type":"array","minItems":1,"maxItems":25,"items":{"type":"string","minLength":3,"maxLength":320}}]},
                    "cc":{"oneOf":[{"type":"string","minLength":3,"maxLength":320},{"type":"array","maxItems":25,"items":{"type":"string","minLength":3,"maxLength":320}}]},
                    "subject":{"type":"string","minLength":1,"maxLength":500},
                    "body":{"type":"string","minLength":1,"maxLength":50000}
                },
                "required":["to","subject","body"]
            }),
            json!({"type":"object","properties":{"id":{"type":"string","minLength":1,"maxLength":512},"isDraft":{"const":true},"mutationPostcondition":{"const":"draft_exists_unsent"}},"required":["id","isDraft","mutationPostcondition"]}),
        ),
        OUTLOOK_CALENDAR_READ => (
            json!({"type":"object","additionalProperties":false,"properties":{"startDateTime":{"type":"string","minLength":1,"maxLength":64},"endDateTime":{"type":"string","minLength":1,"maxLength":64}},"required":["startDateTime","endDateTime"]}),
            list_output(50),
        ),
        OUTLOOK_CALENDAR_DRAFT => (
            json!({"type":"object","additionalProperties":false,"properties":{"subject":{"type":"string","minLength":1,"maxLength":500},"startDateTime":{"type":"string","minLength":1,"maxLength":64},"endDateTime":{"type":"string","minLength":1,"maxLength":64},"timeZone":{"type":"string","maxLength":128},"location":{"type":"string","maxLength":500}},"required":["subject","startDateTime","endDateTime"]}),
            json!({"type":"object","properties":{"localDraft":{"const":true},"eventCreated":{"const":false},"invitationsSent":{"const":false}},"required":["localDraft","eventCreated","invitationsSent"]}),
        ),
        ONEDRIVE_SEARCH => (
            json!({"type":"object","additionalProperties":false,"properties":{"query":{"type":"string","minLength":1,"maxLength":500}},"required":["query"]}),
            list_output(25),
        ),
        ONEDRIVE_READ => (
            json!({"type":"object","additionalProperties":false,"properties":{"itemId":{"type":"string","minLength":1,"maxLength":512}},"required":["itemId"]}),
            json!({"type":"object","properties":{"contentBase64":{"type":"string","maxLength":2796204,"contentEncoding":"base64"},"contentType":{"type":"string","maxLength":255},"size":{"type":"integer","minimum":0,"maximum":2097152}},"required":["contentBase64","contentType","size"]}),
        ),
        ONEDRIVE_WRITE => (write_input(false), write_output()),
        SHAREPOINT_SEARCH => (
            json!({"type":"object","additionalProperties":false,"properties":{"siteId":{"type":"string","minLength":1,"maxLength":512},"query":{"type":"string","minLength":1,"maxLength":500}},"required":["siteId","query"]}),
            list_output(25),
        ),
        SHAREPOINT_READ => (
            json!({"type":"object","additionalProperties":false,"properties":{"siteId":{"type":"string","minLength":1,"maxLength":512},"itemId":{"type":"string","minLength":1,"maxLength":512}},"required":["siteId","itemId"]}),
            json!({"type":"object","properties":{"contentBase64":{"type":"string","maxLength":2796204,"contentEncoding":"base64"},"contentType":{"type":"string","maxLength":255},"size":{"type":"integer","minimum":0,"maximum":2097152}},"required":["contentBase64","contentType","size"]}),
        ),
        SHAREPOINT_WRITE => (write_input(true), write_output()),
        TEAMS_SEARCH => (
            json!({"type":"object","additionalProperties":false,"properties":{"chatId":{"type":"string","minLength":1,"maxLength":512},"query":{"type":"string","minLength":1,"maxLength":500}},"required":["chatId","query"]}),
            list_output(25),
        ),
        TEAMS_DRAFT => (
            json!({"type":"object","additionalProperties":false,"properties":{"chatId":{"type":"string","minLength":1,"maxLength":512},"text":{"type":"string","minLength":1,"maxLength":12000}},"required":["chatId","text"]}),
            json!({"type":"object","properties":{"localDraft":{"const":true},"posted":{"const":false}},"required":["localDraft","posted"]}),
        ),
        _ => (
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object"}),
        ),
    }
}

fn tool(name: &str, risk: &str, description: &str) -> ConnectorTool {
    let (input_schema, output_schema) = schemas(name);
    ConnectorTool {
        name: name.to_string(),
        risk: risk.to_string(),
        description: description.to_string(),
        input_schema,
        output_schema: Some(output_schema),
    }
}

pub(in crate::connectors) fn descriptor(client_id: Option<&str>) -> ConnectorManifest {
    ConnectorManifest {
        manifest_id: MANIFEST_ID.to_string(),
        name: "Microsoft 365".to_string(),
        version: 1,
        transport: "microsoft_graph_https".to_string(),
        auth_method: "oauth_authorization_code_pkce_public_client".to_string(),
        tools: vec![
            tool(OUTLOOK_MAIL_SEARCH, "read", "Search Outlook mail."),
            tool(OUTLOOK_MAIL_READ, "read", "Read an Outlook message."),
            tool(
                OUTLOOK_MAIL_DRAFT,
                "write",
                "Create an Outlook draft after approval; it is never sent.",
            ),
            tool(
                OUTLOOK_CALENDAR_READ,
                "read",
                "Read an Outlook calendar range.",
            ),
            tool(
                OUTLOOK_CALENDAR_DRAFT,
                "draft",
                "Prepare an event locally without creating or inviting anyone.",
            ),
            tool(
                ONEDRIVE_SEARCH,
                "read",
                "Search the signed-in user's OneDrive.",
            ),
            tool(ONEDRIVE_READ, "read", "Read a bounded OneDrive file."),
            tool(
                ONEDRIVE_WRITE,
                "write",
                "Create or replace a bounded OneDrive file after approval and an exact ETag precondition.",
            ),
            tool(
                SHAREPOINT_SEARCH,
                "read",
                "Search files in an exact SharePoint site in a work tenant.",
            ),
            tool(
                SHAREPOINT_READ,
                "read",
                "Read a bounded file from an exact SharePoint site.",
            ),
            tool(
                SHAREPOINT_WRITE,
                "write",
                "Create or replace a bounded file at an exact SharePoint site after approval and an exact ETag precondition.",
            ),
            tool(
                TEAMS_SEARCH,
                "read",
                "Search recent messages in one selected Teams chat.",
            ),
            tool(
                TEAMS_DRAFT,
                "draft",
                "Prepare a Teams message locally without posting it.",
            ),
        ],
        requested_permissions: vec![
            "Sign in and retain an offline session".to_string(),
            "Request each Microsoft Graph read or write grant only when its capability is selected"
                .to_string(),
            "Never request mail-send, meeting-invitation, sharing, or Teams-post permissions"
                .to_string(),
        ],
        base_scopes: base_scopes(),
        operation_grants: manifest_operation_grants(),
        data_destinations: data_routing(),
        project_eligible: true,
        supported: client_id.is_some(),
        availability_reason_code: client_id
            .is_none()
            .then(|| "build_missing_oauth_client".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_never_advertises_send_invite_share_or_post() {
        let names: Vec<String> = descriptor(Some("client"))
            .tools
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        for forbidden in [
            "outlook.mail.send",
            "outlook.calendar.create",
            "outlook.calendar.invite",
            "onedrive.file.delete",
            "onedrive.file.share",
            "sharepoint.file.delete",
            "sharepoint.file.share",
            "teams.chat.post",
        ] {
            assert!(!names.iter().any(|name| name == forbidden));
        }
        assert!(names.contains(&OUTLOOK_MAIL_DRAFT.to_string()));
        assert!(names.contains(&TEAMS_DRAFT.to_string()));
    }

    #[test]
    fn every_tool_exposes_bounded_invocation_and_result_schemas() {
        let tools = descriptor(Some("client")).tools;
        assert_eq!(tools.len(), 13);
        for tool in tools {
            assert_eq!(tool.input_schema["type"], "object");
            assert_eq!(tool.input_schema["additionalProperties"], false);
            assert!(tool.input_schema["properties"]
                .as_object()
                .is_some_and(|p| !p.is_empty()));
            assert!(tool.input_schema["required"].as_array().is_some());
            assert!(tool.output_schema.is_some());
        }
        let write = schemas(ONEDRIVE_WRITE).0;
        assert_eq!(write["oneOf"].as_array().unwrap().len(), 2);
        assert_eq!(write["properties"]["content"]["maxLength"], 4_194_304);
        assert_eq!(
            write["properties"]["contentBase64"]["contentEncoding"],
            "base64"
        );
        assert!(write["allOf"].as_array().unwrap()[0]["then"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "expectedETag"));
    }
}
