use super::super::{
    adapter::{AdapterExecution, ConnectorAdapter, OperationPolicy},
    auth::ConnectorCredential,
    ConnectorCapabilityGrant,
};
use super::contract::*;
use super::discovery;
#[cfg(test)]
use super::graph_response::strip_paging_token;
use super::graph_response::{
    attach_source_citations, object_citation, observed_draft_result, parse_binary, parse_json,
};
use super::http::graph_client;
use crate::foundation::digest::sha256_hex;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::header;
use serde_json::{json, Value};
use url::Url;

pub(in crate::connectors) static MICROSOFT_ADAPTER: Microsoft365Adapter = Microsoft365Adapter;

pub(in crate::connectors) struct Microsoft365Adapter;

impl ConnectorAdapter for Microsoft365Adapter {
    fn operation_for_capability(&self, capability: &str) -> Result<&'static str, String> {
        operation_for_capability(capability)
    }

    fn capabilities_for_operation(&self, operation: &str) -> Vec<&'static str> {
        capabilities_for_operation(operation)
    }

    fn operation_policy(&self, operation: &str) -> Result<OperationPolicy, String> {
        policy(operation)
    }

    fn execute(
        &self,
        credential: Option<&ConnectorCredential>,
        operation: &str,
        arguments: &Value,
    ) -> Result<AdapterExecution, String> {
        if matches!(operation, OUTLOOK_CALENDAR_DRAFT | TEAMS_DRAFT) {
            return local_draft(operation, arguments);
        }
        let credential = credential.ok_or_else(|| "microsoft_credential_missing".to_string())?;
        if credential.manifest_id != MANIFEST_ID {
            return Err("microsoft_credential_manifest_mismatch".to_string());
        }
        require_operation_scopes(&credential.scopes, operation)?;
        if credential.identity_binding_hash.is_none()
            || credential.tenant_id.is_none()
            || credential.account_id.is_none()
        {
            return Err("microsoft_identity_binding_missing".to_string());
        }
        execute_graph(&credential.access_token, operation, arguments)
    }

    fn approval_arguments(&self, operation: &str, arguments: &Value) -> Result<Value, String> {
        if operation == OUTLOOK_MAIL_DRAFT {
            let to = recipient_addresses(arguments, "to")?;
            if to.is_empty() {
                return Err("microsoft_argument_to_required".to_string());
            }
            return Ok(json!({
                "to":to,
                "cc":recipient_addresses(arguments,"cc")?,
                "subject":text_arg(arguments,"subject",500)?,
                "body":content_arg(arguments,"body",50000)?,
            }));
        }
        if matches!(operation, ONEDRIVE_WRITE | SHAREPOINT_WRITE) {
            let (bytes, content_type) = write_payload(arguments)?;
            let path = text_arg(arguments, "path", 2048)?;
            safe_relative_path(path)?;
            let replace = replace_existing(arguments)?;
            let expected_etag = replace
                .then(|| text_arg(arguments, "expectedETag", 256))
                .transpose()?;
            let site_id = (operation == SHAREPOINT_WRITE)
                .then(|| text_arg(arguments, "siteId", 512))
                .transpose()?;
            return Ok(json!({
                "siteId":site_id,
                "path":path,
                "contentBytes":bytes.len(),
                "contentSha256":sha256_hex(&bytes),
                "contentType":content_type,
                "replaceExisting":replace,
                "expectedETag":expected_etag,
            }));
        }
        Ok(arguments.clone())
    }

    fn capability_grants(
        &self,
        granted_scopes: &[String],
        account_kind: Option<&str>,
    ) -> Vec<ConnectorCapabilityGrant> {
        capability_grants(granted_scopes, account_kind)
    }
}

fn policy(operation: &str) -> Result<OperationPolicy, String> {
    let (citation, remote, effectful, classes) = match operation {
        OUTLOOK_MAIL_SEARCH => (
            "graph://outlook/mail/search",
            true,
            false,
            vec!["search_query", "message_metadata"],
        ),
        OUTLOOK_MAIL_READ => (
            "graph://outlook/mail/message",
            true,
            false,
            vec!["message_content"],
        ),
        OUTLOOK_MAIL_DRAFT => (
            "graph://outlook/mail/draft",
            true,
            true,
            vec!["draft_recipients", "draft_content"],
        ),
        OUTLOOK_CALENDAR_READ => (
            "graph://outlook/calendar/view",
            true,
            false,
            vec!["calendar_events"],
        ),
        OUTLOOK_CALENDAR_DRAFT => (
            "local://outlook/calendar/draft",
            false,
            false,
            vec!["event_draft"],
        ),
        ONEDRIVE_SEARCH => (
            "graph://onedrive/search",
            true,
            false,
            vec!["search_query", "file_metadata"],
        ),
        ONEDRIVE_READ => ("graph://onedrive/file", true, false, vec!["file_content"]),
        ONEDRIVE_WRITE => (
            "graph://onedrive/file",
            true,
            true,
            vec!["file_content", "file_destination"],
        ),
        SHAREPOINT_SEARCH => (
            "graph://sharepoint/file/search",
            true,
            false,
            vec!["search_query", "site_identifier", "file_metadata"],
        ),
        SHAREPOINT_RESOLVE => (
            "graph://sharepoint/site/resolve",
            true,
            false,
            vec!["site_url", "site_metadata"],
        ),
        SHAREPOINT_READ => (
            "graph://sharepoint/file",
            true,
            false,
            vec!["file_content", "site_identifier"],
        ),
        SHAREPOINT_WRITE => (
            "graph://sharepoint/file",
            true,
            true,
            vec!["file_content", "site_identifier", "file_destination"],
        ),
        TEAMS_SEARCH => (
            "graph://teams/chat/messages",
            true,
            false,
            vec!["search_query", "chat_messages"],
        ),
        TEAMS_LIST => ("graph://teams/chats", true, false, vec!["chat_metadata"]),
        TEAMS_DRAFT => (
            "local://teams/chat/draft",
            false,
            false,
            vec!["chat_destination", "draft_content"],
        ),
        _ => return Err("microsoft_operation_unsupported".to_string()),
    };
    Ok(OperationPolicy {
        origin: if remote { GRAPH_ORIGIN } else { "local_draft" },
        citation,
        remote,
        effectful,
        data_classes: classes.into_iter().map(str::to_string).collect(),
    })
}

fn operation_for_capability(capability: &str) -> Result<&'static str, String> {
    match capability {
        "find_email" => Ok(OUTLOOK_MAIL_SEARCH),
        "read_email" => Ok(OUTLOOK_MAIL_READ),
        "draft_email" => Ok(OUTLOOK_MAIL_DRAFT),
        "read_calendar" => Ok(OUTLOOK_CALENDAR_READ),
        "draft_calendar_event" => Ok(OUTLOOK_CALENDAR_DRAFT),
        "find_personal_files" => Ok(ONEDRIVE_SEARCH),
        "read_personal_file" => Ok(ONEDRIVE_READ),
        "save_personal_file" => Ok(ONEDRIVE_WRITE),
        "find_team_files" => Ok(SHAREPOINT_SEARCH),
        "read_team_file" => Ok(SHAREPOINT_READ),
        "save_team_file" => Ok(SHAREPOINT_WRITE),
        "find_team_site" => Ok(SHAREPOINT_RESOLVE),
        "list_chats" => Ok(TEAMS_LIST),
        "find_chat_messages" => Ok(TEAMS_SEARCH),
        "draft_chat_message" => Ok(TEAMS_DRAFT),
        _ => Err("connector_task_capability_unsupported".to_string()),
    }
}

fn capabilities_for_operation(operation: &str) -> Vec<&'static str> {
    match operation {
        OUTLOOK_MAIL_SEARCH => vec!["find_email"],
        OUTLOOK_MAIL_READ => vec!["read_email"],
        OUTLOOK_MAIL_DRAFT => vec!["draft_email"],
        OUTLOOK_CALENDAR_READ => vec!["read_calendar"],
        OUTLOOK_CALENDAR_DRAFT => vec!["draft_calendar_event"],
        ONEDRIVE_SEARCH => vec!["find_personal_files"],
        ONEDRIVE_READ => vec!["read_personal_file"],
        ONEDRIVE_WRITE => vec!["save_personal_file"],
        SHAREPOINT_SEARCH => vec!["find_team_site", "find_team_files"],
        SHAREPOINT_READ => vec!["read_team_file"],
        SHAREPOINT_WRITE => vec!["save_team_file"],
        TEAMS_SEARCH => vec!["list_chats", "find_chat_messages"],
        TEAMS_DRAFT => vec!["draft_chat_message"],
        _ => vec![],
    }
}

fn text_arg<'a>(arguments: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("microsoft_argument_{key}_required"))?;
    if value.len() > max || value.chars().any(char::is_control) {
        return Err(format!("microsoft_argument_{key}_invalid"));
    }
    Ok(value)
}

fn content_arg<'a>(arguments: &'a Value, key: &str, max: usize) -> Result<&'a str, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("microsoft_argument_{key}_required"))?;
    if value.len() > max
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(format!("microsoft_argument_{key}_invalid"));
    }
    Ok(value)
}

fn optional_text_arg<'a>(
    arguments: &'a Value,
    key: &str,
    max: usize,
) -> Result<Option<&'a str>, String> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("microsoft_argument_{key}_invalid"))?;
    if value.len() > max || value.chars().any(char::is_control) {
        return Err(format!("microsoft_argument_{key}_invalid"));
    }
    Ok(Some(value))
}

fn graph_url(segments: &[&str]) -> Result<Url, String> {
    let mut url = Url::parse(GRAPH_ROOT).map_err(|_| "microsoft_endpoint_invalid".to_string())?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "microsoft_endpoint_invalid".to_string())?;
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
    }
    Ok(url)
}

fn recipient_addresses(arguments: &Value, key: &str) -> Result<Vec<String>, String> {
    let values: Vec<&str> = match arguments.get(key) {
        None => vec![],
        Some(Value::String(value)) => vec![value.as_str()],
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| format!("microsoft_argument_{key}_invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(format!("microsoft_argument_{key}_invalid")),
    };
    if values.len() > 25 {
        return Err(format!("microsoft_argument_{key}_invalid"));
    }
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim();
            if trimmed.len() < 3
                || trimmed.len() > 320
                || trimmed.chars().any(char::is_whitespace)
                || trimmed.matches('@').count() != 1
            {
                return Err(format!("microsoft_argument_{key}_invalid"));
            }
            Ok(trimmed.to_string())
        })
        .collect()
}

fn recipients(arguments: &Value, key: &str) -> Result<Vec<Value>, String> {
    Ok(recipient_addresses(arguments, key)?
        .into_iter()
        .map(|address| json!({"emailAddress":{"address":address}}))
        .collect())
}

fn replace_existing(arguments: &Value) -> Result<bool, String> {
    match arguments.get("replaceExisting") {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("microsoft_argument_replaceExisting_invalid".to_string()),
    }
}

fn execute_graph(token: &str, operation: &str, args: &Value) -> Result<AdapterExecution, String> {
    let client = graph_client()?;
    let (mut result, partial) = match operation {
        OUTLOOK_MAIL_SEARCH => {
            let query = text_arg(args, "query", 500)?;
            let mut url = graph_url(&["me", "messages"])?;
            url.query_pairs_mut()
                .append_pair("$search", &format!("\"{query}\""))
                .append_pair("$top", "25")
                .append_pair(
                    "$select",
                    "id,subject,from,receivedDateTime,bodyPreview,webLink,isRead",
                );
            parse_json(
                client
                    .get(url)
                    .bearer_auth(token)
                    .header("ConsistencyLevel", "eventual")
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?
        }
        OUTLOOK_MAIL_READ => {
            let id = text_arg(args, "messageId", 512)?;
            let mut url = graph_url(&["me", "messages", id])?;
            url.query_pairs_mut().append_pair(
                "$select",
                "id,subject,from,toRecipients,ccRecipients,receivedDateTime,body,webLink,isRead",
            );
            parse_json(
                client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?
        }
        OUTLOOK_MAIL_DRAFT => {
            let to = recipients(args, "to")?;
            if to.is_empty() {
                return Err("microsoft_argument_to_required".to_string());
            }
            let payload = json!({
                "subject": text_arg(args,"subject",500)?,
                "body":{"contentType":"Text","content":content_arg(args,"body",50000)?},
                "toRecipients":to,
                "ccRecipients":recipients(args,"cc")?
            });
            let (body, partial) = parse_json(
                client
                    .post(graph_url(&["me", "messages"])?)
                    .bearer_auth(token)
                    .json(&payload)
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?;
            (observed_draft_result(&body)?, partial)
        }
        OUTLOOK_CALENDAR_READ => {
            let mut url = graph_url(&["me", "calendar", "calendarView"])?;
            url.query_pairs_mut().append_pair("startDateTime",text_arg(args,"startDateTime",64)?).append_pair("endDateTime",text_arg(args,"endDateTime",64)?).append_pair("$top","50").append_pair("$select","id,subject,start,end,location,organizer,isCancelled,webLink,lastModifiedDateTime");
            parse_json(
                client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?
        }
        ONEDRIVE_SEARCH => {
            let search = format!(
                "search(q='{}')",
                text_arg(args, "query", 500)?.replace('\'', "''")
            );
            let mut url = graph_url(&["me", "drive", "root", &search])?;
            url.query_pairs_mut().append_pair("$top", "25").append_pair(
                "$select",
                "id,name,size,file,folder,lastModifiedDateTime,webUrl,eTag",
            );
            parse_json(
                client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?
        }
        ONEDRIVE_READ => {
            let id = text_arg(args, "itemId", 512)?;
            (
                parse_binary(
                    client
                        .get(graph_url(&["me", "drive", "items", id, "content"])?)
                        .bearer_auth(token)
                        .send()
                        .map_err(|_| "microsoft_request_offline".to_string())?,
                )?,
                false,
            )
        }
        ONEDRIVE_WRITE => write_file(&client, token, &["me", "drive"], args)?,
        SHAREPOINT_SEARCH => {
            let site = text_arg(args, "siteId", 512)?;
            let search = format!(
                "search(q='{}')",
                text_arg(args, "query", 500)?.replace('\'', "''")
            );
            let mut url = graph_url(&["sites", site, "drive", "root", &search])?;
            url.query_pairs_mut().append_pair("$top", "25").append_pair(
                "$select",
                "id,name,size,file,folder,lastModifiedDateTime,webUrl,eTag",
            );
            parse_json(
                client
                    .get(url)
                    .bearer_auth(token)
                    .send()
                    .map_err(|_| "microsoft_request_offline".to_string())?,
            )?
        }
        SHAREPOINT_READ => {
            let site = text_arg(args, "siteId", 512)?;
            let item = text_arg(args, "itemId", 512)?;
            (
                parse_binary(
                    client
                        .get(graph_url(&[
                            "sites", site, "drive", "items", item, "content",
                        ])?)
                        .bearer_auth(token)
                        .send()
                        .map_err(|_| "microsoft_request_offline".to_string())?,
                )?,
                false,
            )
        }
        SHAREPOINT_WRITE => {
            let site = text_arg(args, "siteId", 512)?;
            write_file(&client, token, &["sites", site, "drive"], args)?
        }
        SHAREPOINT_RESOLVE => discovery::resolve_site(&client, token, args)?,
        TEAMS_LIST => discovery::list_chats(&client, token)?,
        TEAMS_SEARCH => search_chat(&client, token, args)?,
        _ => return Err("microsoft_operation_unsupported".to_string()),
    };
    attach_source_citations(operation, args, &mut result)?;
    let operation_policy = policy(operation)?;
    let citation = object_citation(operation, args, &result, operation_policy.citation);
    Ok(AdapterExecution {
        result,
        partial,
        freshness: "live",
        citation,
    })
}

fn local_draft(operation: &str, args: &Value) -> Result<AdapterExecution, String> {
    let result = match operation {
        OUTLOOK_CALENDAR_DRAFT => json!({
            "subject":text_arg(args,"subject",500)?,
            "startDateTime":text_arg(args,"startDateTime",64)?,
            "endDateTime":text_arg(args,"endDateTime",64)?,
            "timeZone":optional_text_arg(args,"timeZone",128)?,
            "location":optional_text_arg(args,"location",500)?,
            "localDraft":true,"eventCreated":false,"invitationsSent":false
        }),
        TEAMS_DRAFT => json!({
            "chatId":text_arg(args,"chatId",512)?,
            "text":content_arg(args,"text",12000)?,
            "localDraft":true,"posted":false
        }),
        _ => return Err("microsoft_operation_unsupported".to_string()),
    };
    let fallback = policy(operation)?.citation;
    let citation = object_citation(operation, args, &result, fallback);
    Ok(AdapterExecution {
        result,
        partial: false,
        freshness: "local_draft",
        citation,
    })
}

fn safe_relative_path(value: &str) -> Result<Vec<&str>, String> {
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
        return Err("microsoft_argument_path_invalid".to_string());
    }
    let segments: Vec<&str> = value.split('/').collect();
    if segments.is_empty()
        || segments.len() > 64
        || segments
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == ".." || part.len() > 255)
    {
        return Err("microsoft_argument_path_invalid".to_string());
    }
    Ok(segments)
}

fn write_payload(arguments: &Value) -> Result<(Vec<u8>, String), String> {
    const MAX_BYTES: usize = 4 * 1024 * 1024;
    let text = arguments.get("content").and_then(Value::as_str);
    let encoded = arguments.get("contentBase64").and_then(Value::as_str);
    let bytes = match (text, encoded) {
        (Some(_), Some(_)) | (None, None) => {
            return Err("microsoft_write_content_choice_invalid".to_string())
        }
        (Some(value), None) => {
            if value.is_empty() || value.len() > MAX_BYTES || value.contains('\0') {
                return Err("microsoft_write_content_invalid".to_string());
            }
            value.as_bytes().to_vec()
        }
        (None, Some(value)) => {
            if value.is_empty() || value.len() > (MAX_BYTES * 4 / 3) + 8 {
                return Err("microsoft_write_content_base64_invalid".to_string());
            }
            let decoded = STANDARD
                .decode(value)
                .map_err(|_| "microsoft_write_content_base64_invalid".to_string())?;
            if decoded.is_empty() || decoded.len() > MAX_BYTES {
                return Err("microsoft_write_content_base64_invalid".to_string());
            }
            decoded
        }
    };
    let content_type = match arguments.get("contentType") {
        None => "application/octet-stream",
        Some(Value::String(value))
            if !value.trim().is_empty()
                && value.len() <= 255
                && value.is_ascii()
                && value.contains('/')
                && !value.chars().any(char::is_control) =>
        {
            value.as_str()
        }
        Some(_) => return Err("microsoft_argument_contentType_invalid".to_string()),
    }
    .to_string();
    Ok((bytes, content_type))
}

fn write_file(
    client: &reqwest::blocking::Client,
    token: &str,
    prefix: &[&str],
    args: &Value,
) -> Result<(Value, bool), String> {
    let path = safe_relative_path(text_arg(args, "path", 2048)?)?;
    let (content, content_type) = write_payload(args)?;
    let replace = replace_existing(args)?;
    let mut segments = prefix.to_vec();
    segments.push("root:");
    for segment in &path[..path.len() - 1] {
        segments.push(segment)
    }
    let last = format!("{}:", path[path.len() - 1]);
    segments.push(&last);
    segments.push("content");
    let mut request = client
        .put(graph_url(&segments)?)
        .bearer_auth(token)
        .header(header::CONTENT_TYPE, content_type);
    request = if replace {
        request.header(header::IF_MATCH, text_arg(args, "expectedETag", 256)?)
    } else {
        request.header(header::IF_NONE_MATCH, "*")
    };
    let (body, partial) = parse_json(
        request
            .body(content.clone())
            .send()
            .map_err(|_| "microsoft_request_offline".to_string())?,
    )?;
    Ok((
        observed_write_result(&body, replace, path[path.len() - 1], content.len() as u64)?,
        partial,
    ))
}

fn observed_write_result(
    body: &Value,
    replace: bool,
    expected_name: &str,
    uploaded_size: u64,
) -> Result<Value, String> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or_else(|| "microsoft_write_postcondition_failed".to_string())?;
    let etag = body
        .get("eTag")
        .and_then(Value::as_str)
        .filter(|etag| !etag.is_empty() && etag.len() <= 512 && !etag.chars().any(char::is_control))
        .ok_or_else(|| "microsoft_write_postcondition_failed".to_string())?;
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| *name == expected_name)
        .ok_or_else(|| "microsoft_write_postcondition_failed".to_string())?;
    let size = body
        .get("size")
        .and_then(Value::as_u64)
        .filter(|size| *size == uploaded_size)
        .ok_or_else(|| "microsoft_write_postcondition_failed".to_string())?;
    Ok(
        json!({"id":id,"name":name,"size":size,"eTag":etag,"webUrl":body.get("webUrl"),"mutationPostcondition":if replace{"file_replaced_at_expected_etag"}else{"new_file_created"}}),
    )
}

fn search_chat(
    client: &reqwest::blocking::Client,
    token: &str,
    args: &Value,
) -> Result<(Value, bool), String> {
    let chat = text_arg(args, "chatId", 512)?;
    let query = text_arg(args, "query", 500)?.to_lowercase();
    let mut url = graph_url(&["chats", chat, "messages"])?;
    url.query_pairs_mut().append_pair("$top", "50");
    let (mut body, partial) = parse_json(
        client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|_| "microsoft_request_offline".to_string())?,
    )?;
    let partial = filter_chat_matches(&mut body, &query, partial)?;
    Ok((body, partial))
}

fn filter_chat_matches(
    body: &mut Value,
    query: &str,
    upstream_partial: bool,
) -> Result<bool, String> {
    let values = body
        .get_mut("value")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "microsoft_response_invalid".to_string())?;
    values.retain(|message| {
        message
            .pointer("/body/content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.to_lowercase().contains(&query))
    });
    let partial = upstream_partial || values.len() > 25;
    values.truncate(25);
    Ok(partial)
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
