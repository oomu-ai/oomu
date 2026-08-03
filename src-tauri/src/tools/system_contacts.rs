use crate::mcp_result::McpToolCallResult;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Arc, Mutex};

const CONTACTS_BACKEND: &str = "contacts";
const DEFAULT_CONTACT_LIMIT: u32 = 20;
const MAX_CONTACT_LIMIT: u32 = 20;
const MAX_SEARCH_CHARACTERS: usize = 128;
const MAX_DISPLAY_NAME_CHARACTERS: usize = 256;
const MAX_VALUES_PER_FIELD: usize = 5;
const MAX_EMAIL_CHARACTERS: usize = 254;
const MAX_PHONE_CHARACTERS: usize = 64;
const CONTACTS_READ_TIMEOUT_SECONDS: u64 = 60;
const AUTHORIZATION_WAIT_SECONDS: u64 = 45;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContactReadRequest {
    pub(crate) max_contacts: u32,
    pub(crate) search_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ContactsAuthorization {
    Authorized,
    Limited,
}

impl ContactsAuthorization {
    fn debug_label(self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::Limited => "limited",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemContact {
    display_name: String,
    emails: Vec<String>,
    phones: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContactsFailure {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) retryable: bool,
}

impl ContactsFailure {
    fn new(code: &str, message: &str, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            retryable,
        }
    }
}

pub(crate) fn contact_request_from_arguments(
    arguments: &Value,
) -> Result<ContactReadRequest, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "Contacts arguments must be a JSON object.".to_string())?;
    let max_contacts = object
        .get("max_contacts")
        .or_else(|| object.get("maxContacts"))
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "Contacts maxContacts must be a positive whole number.".to_string())
        })
        .transpose()?
        .unwrap_or(DEFAULT_CONTACT_LIMIT)
        .clamp(1, MAX_CONTACT_LIMIT);
    let search_text = object
        .get("search_text")
        .or_else(|| object.get("searchText"))
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| "Contacts searchText must be text.".to_string())
        })
        .transpose()?
        .unwrap_or_default();
    let search_text = bounded_text(search_text, MAX_SEARCH_CHARACTERS);
    Ok(ContactReadRequest {
        max_contacts,
        search_text,
    })
}

pub(crate) async fn read_contacts(
    request: ContactReadRequest,
) -> Result<McpToolCallResult, ContactsFailure> {
    let request = ContactReadRequest {
        max_contacts: request.max_contacts.clamp(1, MAX_CONTACT_LIMIT),
        search_text: bounded_text(&request.search_text, MAX_SEARCH_CHARACTERS),
    };
    let search_text = request.search_text.clone();
    let search_present = !search_text.is_empty();
    let native_result = tokio::time::timeout(
        std::time::Duration::from_secs(CONTACTS_READ_TIMEOUT_SECONDS),
        native_contacts(request),
    )
    .await
    .map_err(|_| contacts_read_timeout_failure())
    .and_then(|result| result);
    match native_result {
        Ok((authorization, contacts, truncated)) => {
            if crate::debug_trace_enabled() {
                eprintln!(
                    "OOMU_CONTACTS_NATIVE_RESULT code=contacts_read_ok authorization={} backend={} returned_count={} search_present={}",
                    authorization.debug_label(),
                    CONTACTS_BACKEND,
                    contacts.len(),
                    search_present
                );
            }
            Ok(contacts_success_result(
                CONTACTS_BACKEND,
                "contacts_read_ok",
                Some(authorization),
                &search_text,
                contacts,
                truncated,
                None,
            ))
        }
        Err(failure) => {
            if crate::debug_trace_enabled() {
                eprintln!(
                    "OOMU_CONTACTS_NATIVE_ERROR code={} retryable={} backend={} search_present={}",
                    contacts_debug_error_code(&failure.code),
                    failure.retryable,
                    CONTACTS_BACKEND,
                    search_present
                );
            }
            Err(failure)
        }
    }
}

fn contacts_debug_error_code(code: &str) -> &'static str {
    match code {
        "contacts_read_timeout" => "contacts_read_timeout",
        "contacts_read_failed" => "contacts_read_failed",
        "contacts_authorization_timeout" => "contacts_authorization_timeout",
        "contacts_authorization_cancelled" => "contacts_authorization_cancelled",
        "contacts_permission_denied" => "contacts_permission_denied",
        "contacts_permission_restricted" => "contacts_permission_restricted",
        "contacts_unavailable" => "contacts_unavailable",
        _ => "contacts_unknown_error",
    }
}

fn contacts_read_timeout_failure() -> ContactsFailure {
    ContactsFailure::new(
        "contacts_read_timeout",
        "Contacts took too long to respond. Try again.",
        true,
    )
}

pub(crate) fn contacts_error_result(failure: &ContactsFailure) -> McpToolCallResult {
    let structured = serde_json::json!({
        "backend": CONTACTS_BACKEND,
        "code": failure.code,
        "message": failure.message,
        "retryable": failure.retryable,
        "contacts": [],
    });
    McpToolCallResult {
        content: vec![serde_json::json!({"type": "text", "text": failure.message})],
        structured_content: Some(structured),
        is_error: true,
        meta: None,
        raw: None,
    }
}

pub(crate) fn allows_applescript_fallback(failure: &ContactsFailure) -> bool {
    failure.retryable && failure.code == "contacts_read_failed"
}

pub(crate) fn contacts_applescript_fallback_result(
    primary_failure: &ContactsFailure,
    fallback: McpToolCallResult,
    request: &ContactReadRequest,
) -> McpToolCallResult {
    if fallback.is_error {
        return combined_fallback_failure(primary_failure, "contacts_applescript_failed");
    }
    let Some(items) = fallback
        .structured_content
        .as_ref()
        .and_then(|value| value.get("contacts"))
        .and_then(Value::as_array)
    else {
        return combined_fallback_failure(primary_failure, "contacts_applescript_invalid_result");
    };
    let contacts = items
        .iter()
        .filter_map(system_contact_from_fallback)
        .take(request.max_contacts as usize)
        .collect::<Vec<_>>();
    let truncated = items.len() >= request.max_contacts as usize;
    contacts_success_result(
        "applescript",
        "contacts_read_fallback",
        None,
        &request.search_text,
        contacts,
        truncated,
        Some(primary_failure.code.as_str()),
    )
}

fn combined_fallback_failure(
    primary_failure: &ContactsFailure,
    fallback_code: &str,
) -> McpToolCallResult {
    let message = "Contacts could not be read right now. Try again.";
    let structured = serde_json::json!({
        "backend": "contacts+applescript",
        "code": fallback_code,
        "primaryCode": primary_failure.code,
        "message": message,
        "retryable": true,
        "contacts": [],
    });
    McpToolCallResult {
        content: vec![serde_json::json!({"type": "text", "text": message})],
        structured_content: Some(structured),
        is_error: true,
        meta: None,
        raw: None,
    }
}

fn contacts_success_result(
    backend: &str,
    code: &str,
    authorization: Option<ContactsAuthorization>,
    search_text: &str,
    contacts: Vec<SystemContact>,
    truncated: bool,
    fallback_from: Option<&str>,
) -> McpToolCallResult {
    let returned_count = contacts.len();
    let mut structured = serde_json::json!({
        "backend": backend,
        "code": code,
        "searchText": search_text,
        "contacts": contacts,
        "returnedCount": returned_count,
        "truncated": truncated,
    });
    if let Some(authorization) = authorization {
        structured["authorization"] = serde_json::json!(authorization);
    }
    if let Some(fallback_from) = fallback_from {
        structured["fallbackFrom"] = Value::String(fallback_from.to_string());
    }
    McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": serde_json::to_string_pretty(&structured["contacts"])
                .unwrap_or_else(|_| "[]".to_string()),
        })],
        structured_content: Some(structured),
        is_error: false,
        meta: None,
        raw: None,
    }
}

fn system_contact_from_fallback(value: &Value) -> Option<SystemContact> {
    let object = value.as_object()?;
    let display_name = ["displayName", "name", "organization"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(|value| bounded_text(value, MAX_DISPLAY_NAME_CHARACTERS))
        .filter(|value| !value.is_empty())?;
    Some(SystemContact {
        display_name,
        emails: bounded_values(object.get("emails"), MAX_EMAIL_CHARACTERS),
        phones: bounded_values(object.get("phones"), MAX_PHONE_CHARACTERS),
    })
}

fn bounded_values(value: Option<&Value>, maximum_characters: usize) -> Vec<String> {
    let mut values = Vec::new();
    for value in value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let bounded = bounded_text(value, maximum_characters);
        if !bounded.is_empty() && !values.contains(&bounded) {
            values.push(bounded);
        }
        if values.len() >= MAX_VALUES_PER_FIELD {
            break;
        }
    }
    values
}

fn bounded_text(value: &str, maximum_characters: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum_characters)
        .collect::<String>()
        .trim()
        .to_string()
}

fn finish_collected_contacts(
    contacts: &Arc<Mutex<Vec<SystemContact>>>,
    max_contacts: u32,
) -> Result<(Vec<SystemContact>, bool), ContactsFailure> {
    // The Objective-C enumeration block owns another Arc while it is alive. Reading the
    // completed, synchronously populated collection must therefore not depend on unique Arc
    // ownership.
    let mut contacts = contacts.lock().map_err(|_| {
        ContactsFailure::new(
            "contacts_read_failed",
            "Contacts could not be read right now. Try again.",
            true,
        )
    })?;
    let truncated = contacts.len() > max_contacts as usize;
    contacts.truncate(max_contacts as usize);
    Ok((contacts.clone(), truncated))
}

#[cfg(target_os = "macos")]
async fn native_contacts(
    request: ContactReadRequest,
) -> Result<(ContactsAuthorization, Vec<SystemContact>, bool), ContactsFailure> {
    tokio::task::spawn_blocking(move || native_contacts_blocking(request))
        .await
        .map_err(|_| {
            ContactsFailure::new(
                "contacts_read_failed",
                "Contacts could not be read right now. Try again.",
                true,
            )
        })?
}

#[cfg(target_os = "macos")]
fn native_contacts_blocking(
    request: ContactReadRequest,
) -> Result<(ContactsAuthorization, Vec<SystemContact>, bool), ContactsFailure> {
    use block2::RcBlock;
    use objc2::rc::autoreleasepool;
    use objc2::runtime::{Bool, ProtocolObject};
    use objc2::AnyThread;
    use objc2_contacts::{
        CNAuthorizationStatus, CNContact, CNContactEmailAddressesKey, CNContactFetchRequest,
        CNContactFormatter, CNContactFormatterStyle, CNContactOrganizationNameKey,
        CNContactPhoneNumbersKey, CNContactSortOrder, CNContactStore, CNEntityType,
        CNKeyDescriptor, CNPhoneNumber,
    };
    use objc2_foundation::{NSArray, NSError, NSString};
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::Duration;

    fn authorize(store: &CNContactStore) -> Result<ContactsAuthorization, ContactsFailure> {
        let mut status =
            unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
        if status == CNAuthorizationStatus::NotDetermined {
            let (sender, receiver) = mpsc::channel::<bool>();
            let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                let _ = sender.send(granted.as_bool());
            });
            unsafe {
                store.requestAccessForEntityType_completionHandler(
                    CNEntityType::Contacts,
                    &completion,
                );
            }
            let granted = receiver
                .recv_timeout(Duration::from_secs(AUTHORIZATION_WAIT_SECONDS))
                .map_err(|_| {
                    ContactsFailure::new(
                        "contacts_authorization_timeout",
                        "Contacts took too long to respond. Try again.",
                        true,
                    )
                })?;
            status =
                unsafe { CNContactStore::authorizationStatusForEntityType(CNEntityType::Contacts) };
            if granted && status == CNAuthorizationStatus::NotDetermined {
                status = CNAuthorizationStatus::Authorized;
            } else if !granted && status == CNAuthorizationStatus::NotDetermined {
                return Err(ContactsFailure::new(
                    "contacts_authorization_cancelled",
                    "Contacts did not finish the access request. Try again.",
                    true,
                ));
            }
        }
        if status == CNAuthorizationStatus::Authorized {
            return Ok(ContactsAuthorization::Authorized);
        }
        if status == CNAuthorizationStatus::Limited {
            return Ok(ContactsAuthorization::Limited);
        }
        if status == CNAuthorizationStatus::Denied {
            return Err(ContactsFailure::new(
                "contacts_permission_denied",
                "Contacts access is off. Allow OOMU in System Settings, then try again.",
                false,
            ));
        }
        if status == CNAuthorizationStatus::Restricted {
            return Err(ContactsFailure::new(
                "contacts_permission_restricted",
                "Contacts access is restricted on this Mac.",
                false,
            ));
        }
        Err(ContactsFailure::new(
            "contacts_authorization_cancelled",
            "Contacts did not finish the access request. Try again.",
            true,
        ))
    }

    unsafe fn contact_from_native(contact: &CNContact) -> Option<SystemContact> {
        let display_name =
            CNContactFormatter::stringFromContact_style(contact, CNContactFormatterStyle::FullName)
                .map(|value| bounded_text(&value.to_string(), MAX_DISPLAY_NAME_CHARACTERS))
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    let organization = contact.organizationName().to_string();
                    let organization = bounded_text(&organization, MAX_DISPLAY_NAME_CHARACTERS);
                    (!organization.is_empty()).then_some(organization)
                })?;
        let emails = contact
            .emailAddresses()
            .to_vec()
            .into_iter()
            .map(|entry| entry.value().to_string())
            .map(|value| bounded_text(&value, MAX_EMAIL_CHARACTERS))
            .filter(|value| !value.is_empty())
            .take(MAX_VALUES_PER_FIELD)
            .collect::<Vec<_>>();
        let phones = contact
            .phoneNumbers()
            .to_vec()
            .into_iter()
            .map(|entry| entry.value().stringValue().to_string())
            .map(|value| bounded_text(&value, MAX_PHONE_CHARACTERS))
            .filter(|value| !value.is_empty())
            .take(MAX_VALUES_PER_FIELD)
            .collect::<Vec<_>>();
        Some(SystemContact {
            display_name,
            emails,
            phones,
        })
    }

    autoreleasepool(|_| unsafe {
        let store = CNContactStore::new();
        let authorization = authorize(&store)?;

        let name_descriptor = CNContactFormatter::descriptorForRequiredKeysForStyle(
            CNContactFormatterStyle::FullName,
        );
        let organization_key: &ProtocolObject<dyn CNKeyDescriptor> =
            ProtocolObject::from_ref(CNContactOrganizationNameKey);
        let email_key: &ProtocolObject<dyn CNKeyDescriptor> =
            ProtocolObject::from_ref(CNContactEmailAddressesKey);
        let phone_key: &ProtocolObject<dyn CNKeyDescriptor> =
            ProtocolObject::from_ref(CNContactPhoneNumbersKey);
        let keys =
            NSArray::from_slice(&[&*name_descriptor, organization_key, email_key, phone_key]);
        let fetch_request =
            CNContactFetchRequest::initWithKeysToFetch(CNContactFetchRequest::alloc(), &keys);
        fetch_request.setSortOrder(CNContactSortOrder::UserDefault);
        if !request.search_text.is_empty() {
            let search = NSString::from_str(&request.search_text);
            let predicate = if request.search_text.contains('@') {
                CNContact::predicateForContactsMatchingEmailAddress(&search)
            } else if request
                .search_text
                .chars()
                .filter(|character| character.is_ascii_digit())
                .count()
                >= 3
            {
                CNPhoneNumber::phoneNumberWithStringValue(&search)
                    .map(|phone| CNContact::predicateForContactsMatchingPhoneNumber(&phone))
                    .unwrap_or_else(|| CNContact::predicateForContactsMatchingName(&search))
            } else {
                CNContact::predicateForContactsMatchingName(&search)
            };
            fetch_request.setPredicate(Some(&predicate));
        }

        let contacts = Arc::new(Mutex::new(Vec::<SystemContact>::new()));
        let collected = Arc::clone(&contacts);
        let stop_after = request.max_contacts as usize + 1;
        let block = RcBlock::new(
            move |contact_pointer: NonNull<CNContact>, stop_pointer: NonNull<Bool>| {
                let Some(contact) = contact_pointer.as_ptr().as_ref() else {
                    return;
                };
                let Some(contact) = contact_from_native(contact) else {
                    return;
                };
                if let Ok(mut contacts) = collected.lock() {
                    contacts.push(contact);
                    if contacts.len() >= stop_after {
                        stop_pointer.as_ptr().write(Bool::YES);
                    }
                }
            },
        );
        let mut native_error = None;
        let succeeded = store.enumerateContactsWithFetchRequest_error_usingBlock(
            &fetch_request,
            Some(&mut native_error),
            &block,
        );
        if !succeeded || native_error.is_some() {
            return Err(ContactsFailure::new(
                "contacts_read_failed",
                "Contacts could not be read right now. Try again.",
                true,
            ));
        }
        let (contacts, truncated) = finish_collected_contacts(&contacts, request.max_contacts)?;
        Ok((authorization, contacts, truncated))
    })
}

#[cfg(not(target_os = "macos"))]
async fn native_contacts(
    _request: ContactReadRequest,
) -> Result<(ContactsAuthorization, Vec<SystemContact>, bool), ContactsFailure> {
    Err(ContactsFailure::new(
        "contacts_unavailable",
        "Contacts access is available only in the OOMU app on macOS.",
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_limits_and_search_are_bounded() {
        let request = contact_request_from_arguments(&serde_json::json!({
            "maxContacts": 500,
            "searchText": "x".repeat(300),
        }))
        .unwrap();
        assert_eq!(request.max_contacts, 20);
        assert_eq!(request.search_text.chars().count(), 128);
        assert!(
            contact_request_from_arguments(&serde_json::json!({"maxContacts": "all"})).is_err()
        );
    }

    #[test]
    fn results_are_minimal_typed_and_never_raw() {
        let result = contacts_success_result(
            CONTACTS_BACKEND,
            "contacts_read_ok",
            Some(ContactsAuthorization::Authorized),
            "Ada",
            vec![SystemContact {
                display_name: "Ada Lovelace".to_string(),
                emails: vec!["ada@example.com".to_string()],
                phones: vec!["555-0100".to_string()],
            }],
            false,
            None,
        );
        assert!(!result.is_error);
        assert!(result.raw.is_none());
        let structured = result.structured_content.unwrap();
        assert_eq!(structured["code"], "contacts_read_ok");
        assert_eq!(
            structured["contacts"][0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["displayName", "emails", "phones"]
        );

        let error = contacts_error_result(&ContactsFailure::new(
            "contacts_permission_denied",
            "Contacts access is off.",
            false,
        ));
        assert!(error.is_error);
        assert!(error.raw.is_none());
        assert_eq!(
            error.structured_content.unwrap()["code"],
            "contacts_permission_denied"
        );
    }

    #[test]
    fn debug_labels_are_typed_and_never_echo_untrusted_values() {
        assert_eq!(
            ContactsAuthorization::Authorized.debug_label(),
            "authorized"
        );
        assert_eq!(ContactsAuthorization::Limited.debug_label(), "limited");
        assert_eq!(
            contacts_debug_error_code("contacts_permission_denied"),
            "contacts_permission_denied"
        );
        assert_eq!(
            contacts_debug_error_code("private contact content\n"),
            "contacts_unknown_error"
        );
    }

    #[test]
    fn privacy_and_authorization_failures_never_trigger_automation() {
        for code in [
            "contacts_permission_denied",
            "contacts_permission_restricted",
            "contacts_authorization_timeout",
            "contacts_authorization_cancelled",
        ] {
            let failure = ContactsFailure::new(code, "Contacts access did not complete.", true);
            assert!(!allows_applescript_fallback(&failure), "{code}");
        }
        let transient =
            ContactsFailure::new("contacts_read_failed", "Contacts could not be read.", true);
        assert!(allows_applescript_fallback(&transient));
    }

    #[test]
    fn fallback_is_labeled_bounded_and_strips_extra_fields() {
        let primary =
            ContactsFailure::new("contacts_read_failed", "Contacts could not be read.", true);
        let fallback = McpToolCallResult {
            content: vec![serde_json::json!({"type": "text", "text": "raw-canary"})],
            structured_content: Some(serde_json::json!({
                "contacts": [{
                    "name": "Ada Lovelace",
                    "organization": "Secret organization",
                    "emails": ["ada@example.com"],
                    "phones": ["555-0100"],
                    "notes": "must-not-escape"
                }]
            })),
            is_error: false,
            meta: None,
            raw: Some(serde_json::json!({"jsonrpc": "2.0", "raw": "raw-canary"})),
        };
        let result = contacts_applescript_fallback_result(
            &primary,
            fallback,
            &ContactReadRequest {
                max_contacts: 20,
                search_text: "Ada".to_string(),
            },
        );
        assert!(result.raw.is_none());
        let serialized = result.structured_content.unwrap().to_string();
        assert!(serialized.contains("contacts_read_fallback"));
        assert!(!serialized.contains("Secret organization"));
        assert!(!serialized.contains("must-not-escape"));
        assert!(!serialized.contains("raw-canary"));
    }

    #[test]
    fn completed_native_collection_does_not_require_unique_callback_ownership() {
        let contacts = Arc::new(Mutex::new(vec![
            SystemContact {
                display_name: "Maya Allan".to_string(),
                emails: vec!["maya@example.com".to_string()],
                phones: vec![],
            },
            SystemContact {
                display_name: "Second Result".to_string(),
                emails: vec![],
                phones: vec![],
            },
        ]));
        let callback_capture = Arc::clone(&contacts);

        let (finished, truncated) = finish_collected_contacts(&contacts, 1).unwrap();

        assert_eq!(Arc::strong_count(&contacts), 2);
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].display_name, "Maya Allan");
        assert!(truncated);
        drop(callback_capture);
    }

    #[test]
    fn overall_contact_read_deadline_is_typed_and_does_not_launch_a_second_reader() {
        assert_eq!(CONTACTS_READ_TIMEOUT_SECONDS, 60);
        assert!(AUTHORIZATION_WAIT_SECONDS < CONTACTS_READ_TIMEOUT_SECONDS);

        let timeout = contacts_read_timeout_failure();
        assert_eq!(
            contacts_error_result(&timeout).structured_content.unwrap()["code"],
            "contacts_read_timeout"
        );
        assert!(!allows_applescript_fallback(&timeout));
    }
}
