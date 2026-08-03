use super::{auth::graph_status_code, contract::*};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use reqwest::{header, StatusCode};
use serde_json::{json, Value};
use std::io::Read;

const MAX_JSON_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn bounded_response_bytes(
    response: &mut reqwest::blocking::Response,
    max_bytes: u64,
    unreadable_code: &str,
    too_large_code: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(too_large_code.to_string());
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable_code.to_string())?;
    if bytes.len() as u64 > max_bytes {
        return Err(too_large_code.to_string());
    }
    Ok(bytes)
}

pub(super) fn parse_json(
    mut response: reqwest::blocking::Response,
) -> Result<(Value, bool), String> {
    let status = response.status();
    if !status.is_success() {
        return Err(if status == StatusCode::PRECONDITION_FAILED {
            "microsoft_write_precondition_failed".to_string()
        } else {
            graph_status_code(status)
        });
    }
    let bytes = bounded_response_bytes(
        &mut response,
        MAX_JSON_BYTES,
        "microsoft_response_unreadable",
        "microsoft_response_too_large",
    )?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "microsoft_response_invalid".to_string())?;
    Ok(strip_paging_token(value))
}

pub(super) fn strip_paging_token(mut value: Value) -> (Value, bool) {
    let partial = value.get("@odata.nextLink").is_some();
    if let Some(object) = value.as_object_mut() {
        object.remove("@odata.nextLink");
    }
    (value, partial)
}

pub(super) fn parse_binary(mut response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(graph_status_code(status));
    }
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 255 && value.is_ascii())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = bounded_response_bytes(
        &mut response,
        MAX_FILE_BYTES,
        "microsoft_response_unreadable",
        "microsoft_file_read_too_large",
    )?;
    Ok(json!({
        "contentBase64": STANDARD.encode(&bytes),
        "contentType": content_type,
        "size": bytes.len()
    }))
}

pub(super) fn observed_draft_result(body: &Value) -> Result<Value, String> {
    if body.get("isDraft").and_then(Value::as_bool) != Some(true) {
        return Err("microsoft_draft_postcondition_failed".to_string());
    }
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or_else(|| "microsoft_draft_postcondition_failed".to_string())?;
    Ok(json!({
        "id":id,
        "subject":body.get("subject"),
        "webLink":body.get("webLink"),
        "isDraft":true,
        "mutationPostcondition":"draft_exists_unsent"
    }))
}

fn encoded_citation_id(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn bounded_encoded_id(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control))
        .then(|| encoded_citation_id(value))
}

pub(super) fn attach_source_citations(
    operation: &str,
    args: &Value,
    result: &mut Value,
) -> Result<(), String> {
    if !matches!(
        operation,
        OUTLOOK_MAIL_SEARCH
            | OUTLOOK_CALENDAR_READ
            | ONEDRIVE_SEARCH
            | SHAREPOINT_SEARCH
            | TEAMS_LIST
            | TEAMS_SEARCH
    ) {
        return Ok(());
    }
    let values = result
        .get("value")
        .and_then(Value::as_array)
        .filter(|values| values.len() <= 50)
        .ok_or_else(|| "microsoft_response_invalid".to_string())?;
    let site = args
        .get("siteId")
        .and_then(Value::as_str)
        .and_then(bounded_encoded_id);
    let chat = args
        .get("chatId")
        .and_then(Value::as_str)
        .and_then(bounded_encoded_id);
    let citations: Result<Vec<String>, String> = values
        .iter()
        .map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .and_then(bounded_encoded_id)
                .ok_or_else(|| "microsoft_response_identity_missing".to_string())?;
            match operation {
                OUTLOOK_MAIL_SEARCH => Ok(format!("graph://outlook/mail/message/{id}")),
                OUTLOOK_CALENDAR_READ => Ok(format!("graph://outlook/calendar/event/{id}")),
                ONEDRIVE_SEARCH => Ok(format!("graph://onedrive/item/{id}")),
                SHAREPOINT_SEARCH => site
                    .as_ref()
                    .map(|site| format!("graph://sharepoint/site/{site}/item/{id}"))
                    .ok_or_else(|| "microsoft_argument_siteId_required".to_string()),
                TEAMS_LIST => Ok(format!("graph://teams/chat/{id}")),
                TEAMS_SEARCH => chat
                    .as_ref()
                    .map(|chat| format!("graph://teams/chat/{chat}/message/{id}"))
                    .ok_or_else(|| "microsoft_argument_chatId_required".to_string()),
                _ => Err("microsoft_operation_unsupported".to_string()),
            }
        })
        .collect();
    result
        .as_object_mut()
        .ok_or_else(|| "microsoft_response_invalid".to_string())?
        .insert("sourceCitations".to_string(), json!(citations?));
    Ok(())
}

pub(super) fn object_citation(
    operation: &str,
    args: &Value,
    result: &Value,
    fallback: &str,
) -> String {
    let argument = |key: &str| args.get(key).and_then(Value::as_str);
    let returned_id = || result.get("id").and_then(Value::as_str);
    match operation {
        OUTLOOK_MAIL_READ => argument("messageId")
            .map(|id| format!("graph://outlook/mail/message/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        OUTLOOK_MAIL_DRAFT => returned_id()
            .map(|id| format!("graph://outlook/mail/draft/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        ONEDRIVE_READ => argument("itemId")
            .map(|id| format!("graph://onedrive/item/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        ONEDRIVE_WRITE => returned_id()
            .map(|id| format!("graph://onedrive/item/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        SHAREPOINT_READ => match (argument("siteId"), argument("itemId")) {
            (Some(site), Some(item)) => format!(
                "graph://sharepoint/site/{}/item/{}",
                encoded_citation_id(site),
                encoded_citation_id(item)
            ),
            _ => fallback.to_string(),
        },
        SHAREPOINT_WRITE => match (argument("siteId"), returned_id()) {
            (Some(site), Some(item)) => format!(
                "graph://sharepoint/site/{}/item/{}",
                encoded_citation_id(site),
                encoded_citation_id(item)
            ),
            _ => fallback.to_string(),
        },
        SHAREPOINT_SEARCH => argument("siteId")
            .map(|site| {
                format!(
                    "graph://sharepoint/site/{}/file/search",
                    encoded_citation_id(site)
                )
            })
            .unwrap_or_else(|| fallback.to_string()),
        SHAREPOINT_RESOLVE => returned_id()
            .map(|id| format!("graph://sharepoint/site/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        TEAMS_LIST => fallback.to_string(),
        TEAMS_SEARCH => argument("chatId")
            .map(|id| format!("graph://teams/chat/{}", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        TEAMS_DRAFT => argument("chatId")
            .map(|id| format!("local://teams/chat/{}/draft", encoded_citation_id(id)))
            .unwrap_or_else(|| fallback.to_string()),
        _ => fallback.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_list_item_requires_an_exact_object_citation() {
        let mut result = json!({"value":[{"id":"one"},{"id":"two"}]});
        attach_source_citations(ONEDRIVE_SEARCH, &json!({}), &mut result).unwrap();
        assert_eq!(result["sourceCitations"].as_array().unwrap().len(), 2);

        let mut missing = json!({"value":[{"name":"no-id"}]});
        assert_eq!(
            attach_source_citations(ONEDRIVE_SEARCH, &json!({}), &mut missing).unwrap_err(),
            "microsoft_response_identity_missing"
        );
    }
}
