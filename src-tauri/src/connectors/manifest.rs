use super::{ConnectorCapabilityGrant, ConnectorManifest, ConnectorOperationGrant, ConnectorTool};
use serde_json::json;

const GOOGLE_CLIENT_ID: Option<&str> = option_env!("OOMU_GOOGLE_OAUTH_CLIENT_ID");
include!(concat!(
    env!("OUT_DIR"),
    "/google_oauth_client_credential.rs"
));
const SLACK_CLIENT_ID: Option<&str> = option_env!("OOMU_SLACK_OAUTH_CLIENT_ID");
const MICROSOFT_CLIENT_ID: Option<&str> = option_env!("OOMU_MICROSOFT_OAUTH_CLIENT_ID");
const GOOGLE_EMAIL_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.email";
const GOOGLE_PROFILE_SCOPE: &str = "https://www.googleapis.com/auth/userinfo.profile";
const GOOGLE_BASE_SCOPES: &[&str] = &[GOOGLE_EMAIL_SCOPE, GOOGLE_PROFILE_SCOPE];
const GOOGLE_GMAIL_READ: &str = "https://www.googleapis.com/auth/gmail.readonly";
const GOOGLE_GMAIL_DRAFT: &str = "https://www.googleapis.com/auth/gmail.compose";
const GOOGLE_CALENDAR_READ: &str = "https://www.googleapis.com/auth/calendar.readonly";
const GOOGLE_CALENDAR_WRITE: &str = "https://www.googleapis.com/auth/calendar.events";
const GOOGLE_DRIVE_READ: &str = "https://www.googleapis.com/auth/drive.readonly";
const SLACK_READ_SCOPES: &[&str] = &[
    "channels:history",
    "channels:read",
    "groups:history",
    "groups:read",
    "im:history",
    "mpim:history",
    "search:read",
];
const SLACK_MESSAGING_BOT_SCOPES: &[&str] = &[
    "app_mentions:read",
    "channels:history",
    "channels:read",
    "groups:history",
    "groups:read",
    "im:history",
    "im:read",
    "mpim:history",
    "mpim:read",
    "chat:write",
];
pub(crate) const SLACK_MESSAGING_OPERATION: &str = "slack.messaging";

pub(super) fn oauth_base_scopes(manifest_id: &str) -> Vec<String> {
    let scopes: &[&str] = match manifest_id {
        "google_workspace" => GOOGLE_BASE_SCOPES,
        "slack" => SLACK_READ_SCOPES,
        super::microsoft365::MANIFEST_ID => return super::microsoft365::base_scopes(),
        _ => &[],
    };
    scopes.iter().map(|scope| (*scope).to_string()).collect()
}

fn canonical_google_scope(scope: &str) -> Option<&str> {
    match scope.trim() {
        "openid" => None,
        "email" | GOOGLE_EMAIL_SCOPE => Some(GOOGLE_EMAIL_SCOPE),
        "profile" | GOOGLE_PROFILE_SCOPE => Some(GOOGLE_PROFILE_SCOPE),
        "" => None,
        scope => Some(scope),
    }
}

pub(super) fn normalize_google_scopes(scopes: &[String]) -> Vec<String> {
    let mut normalized = Vec::with_capacity(scopes.len());
    for scope in scopes {
        let Some(scope) = canonical_google_scope(scope) else {
            continue;
        };
        if !normalized.iter().any(|existing| existing == scope) {
            normalized.push(scope.to_string());
        }
    }
    normalized
}

pub(super) fn google_scopes_include(requested: &[String], granted: &[String]) -> bool {
    let requested = normalize_google_scopes(requested);
    let granted = normalize_google_scopes(granted);
    requested.iter().all(|scope| granted.contains(scope))
}

pub(super) fn slack_read_scopes() -> Vec<String> {
    SLACK_READ_SCOPES
        .iter()
        .map(|scope| (*scope).to_string())
        .collect()
}

pub(super) fn slack_bot_scopes(requested_operations: &[String]) -> Result<Vec<String>, String> {
    if requested_operations.is_empty() {
        return Ok(Vec::new());
    }
    if requested_operations
        .iter()
        .any(|operation| operation != SLACK_MESSAGING_OPERATION)
    {
        return Err("connector_incremental_consent_unsupported".to_string());
    }
    Ok(SLACK_MESSAGING_BOT_SCOPES
        .iter()
        .map(|scope| (*scope).to_string())
        .collect())
}

pub(super) fn slack_requested_scopes(
    requested_operations: &[String],
) -> Result<Vec<String>, String> {
    let mut scopes = slack_read_scopes();
    for scope in slack_bot_scopes(requested_operations)? {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}

fn google_grant(
    operation: &str,
    purpose_code: &str,
    access_level: &str,
    required_scopes: &[&str],
    remote_mutation: bool,
) -> ConnectorOperationGrant {
    ConnectorOperationGrant {
        operation: operation.to_string(),
        purpose_code: purpose_code.to_string(),
        required_scopes: required_scopes
            .iter()
            .map(|scope| (*scope).to_string())
            .collect(),
        access_level: access_level.to_string(),
        remote_mutation,
        admin_consent_required: false,
        available: true,
        unavailable_reason_code: None,
    }
}

pub(super) fn google_operation_grants() -> Vec<ConnectorOperationGrant> {
    vec![
        google_grant(
            "gmail.search",
            "connector.google.consent.gmail_read",
            "read",
            &[GOOGLE_GMAIL_READ],
            false,
        ),
        google_grant(
            "gmail.read",
            "connector.google.consent.gmail_read",
            "read",
            &[GOOGLE_GMAIL_READ],
            false,
        ),
        google_grant(
            "gmail.draft",
            "connector.google.consent.gmail_draft",
            "write",
            &[GOOGLE_GMAIL_DRAFT],
            true,
        ),
        google_grant(
            "calendar.read",
            "connector.google.consent.calendar_read",
            "read",
            &[GOOGLE_CALENDAR_READ],
            false,
        ),
        google_grant(
            "calendar.create",
            "connector.google.consent.calendar_write",
            "write",
            &[GOOGLE_CALENDAR_WRITE],
            true,
        ),
        google_grant(
            "calendar.update",
            "connector.google.consent.calendar_write",
            "write",
            &[GOOGLE_CALENDAR_WRITE],
            true,
        ),
        google_grant(
            "drive.search",
            "connector.google.consent.drive_read",
            "read",
            &[GOOGLE_DRIVE_READ],
            false,
        ),
        google_grant(
            "drive.read",
            "connector.google.consent.drive_read",
            "read",
            &[GOOGLE_DRIVE_READ],
            false,
        ),
        google_grant(
            "drive.export",
            "connector.google.consent.drive_read",
            "read",
            &[GOOGLE_DRIVE_READ],
            false,
        ),
    ]
}

pub(super) fn google_requested_scopes(
    requested_operations: &[String],
) -> Result<Vec<String>, String> {
    let mut scopes = oauth_base_scopes("google_workspace");
    let grants = google_operation_grants();
    for operation in requested_operations {
        let grant = grants
            .iter()
            .find(|grant| grant.operation == *operation)
            .ok_or_else(|| "connector_incremental_consent_unsupported".to_string())?;
        for scope in &grant.required_scopes {
            if !scopes.contains(scope) {
                scopes.push(scope.clone());
            }
        }
    }
    Ok(normalize_google_scopes(&scopes))
}

pub(super) fn google_required_scopes(operation: &str) -> Result<Vec<String>, String> {
    google_operation_grants()
        .into_iter()
        .find(|grant| grant.operation == operation)
        .map(|grant| grant.required_scopes)
        .ok_or_else(|| "connector_incremental_consent_unsupported".to_string())
}

pub(super) fn google_capability_grants(granted_scopes: &[String]) -> Vec<ConnectorCapabilityGrant> {
    google_operation_grants()
        .into_iter()
        .map(|grant| ConnectorCapabilityGrant {
            capability_id: grant.operation,
            access_level: grant.access_level,
            granted: grant
                .required_scopes
                .iter()
                .all(|scope| granted_scopes.contains(scope)),
            required_scopes: grant.required_scopes,
            admin_consent_required: grant.admin_consent_required,
            remote_mutation: grant.remote_mutation,
            available: grant.available,
            unavailable_reason_code: grant.unavailable_reason_code,
        })
        .collect()
}

pub(super) fn oauth_client_id(manifest_id: &str) -> Option<&'static str> {
    match manifest_id {
        "google_workspace" => GOOGLE_CLIENT_ID,
        "slack" => SLACK_CLIENT_ID,
        super::microsoft365::MANIFEST_ID => MICROSOFT_CLIENT_ID,
        _ => None,
    }
}

pub(super) fn google_oauth_client_secret() -> Option<&'static str> {
    GOOGLE_CLIENT_SECRET
}

fn tool(name: &str, risk: &str, description: &str) -> ConnectorTool {
    ConnectorTool {
        name: name.to_string(),
        risk: risk.to_string(),
        description: description.to_string(),
        input_schema: json!({"type":"object"}),
        output_schema: None,
    }
}

fn slack_availability_reason() -> Option<String> {
    if SLACK_CLIENT_ID.is_none() {
        Some("build_missing_oauth_client".to_string())
    } else {
        None
    }
}

fn slack_messaging_availability_reason() -> Option<String> {
    (!super::oauth_broker::configured()).then(|| "build_missing_oauth_broker".to_string())
}

fn slack_manifest() -> ConnectorManifest {
    ConnectorManifest {
        manifest_id: "slack".to_string(),
        name: "Slack".to_string(),
        version: 1,
        transport: "https_api".to_string(),
        auth_method: "oauth_authorization_code_pkce".to_string(),
        tools: vec![
            tool("slack.search", "read", "Search channels and threads."),
            tool("slack.thread", "read", "Read an exact channel thread."),
            tool(
                "slack.draft",
                "write",
                "Prepare a message without posting it.",
            ),
            tool("slack.post", "write", "Post an approved message."),
        ],
        requested_permissions: vec!["Search and read channels and threads".to_string()],
        base_scopes: oauth_base_scopes("slack"),
        operation_grants: vec![ConnectorOperationGrant {
            operation: SLACK_MESSAGING_OPERATION.to_string(),
            purpose_code: "connector.slack.consent.messaging".to_string(),
            access_level: "write".to_string(),
            required_scopes: SLACK_MESSAGING_BOT_SCOPES
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            admin_consent_required: false,
            remote_mutation: true,
            available: super::oauth_broker::configured(),
            unavailable_reason_code: slack_messaging_availability_reason(),
        }],
        data_destinations: vec!["https://slack.com".to_string()],
        project_eligible: true,
        supported: SLACK_CLIENT_ID.is_some(),
        availability_reason_code: slack_availability_reason(),
    }
}

pub(super) fn manifests() -> Vec<ConnectorManifest> {
    vec![
        ConnectorManifest {
            manifest_id: "apple_apps".to_string(),
            name: "Apple Apps".to_string(),
            version: 1,
            transport: "native_macos".to_string(),
            auth_method: "macos_permission".to_string(),
            tools: vec![
                tool("calendar.read", "read", "Read events from Apple Calendar."),
                tool(
                    "contacts.read",
                    "read",
                    "Find contacts by name, email, or phone when asked.",
                ),
                tool("mail.read", "read", "Read selected Apple Mail messages."),
                tool(
                    "photos.read",
                    "read",
                    "Read bounded photo metadata from Apple Photos.",
                ),
                tool(
                    "music.read",
                    "read",
                    "Read bounded newest-added song metadata from Apple Music when asked.",
                ),
                tool(
                    "calendar.change",
                    "write",
                    "Create or change an event after approval.",
                ),
            ],
            requested_permissions: vec![
                "Automation access to the selected Apple applications".to_string(),
                "Read only the Calendar range you ask OOMU to review".to_string(),
                "Read only the contacts you ask OOMU to find".to_string(),
                "Read-only access to bounded metadata in Apple Photos".to_string(),
                "Read only bounded song metadata from Apple Music when you ask".to_string(),
            ],
            base_scopes: vec![],
            operation_grants: vec![],
            data_destinations: vec!["Only the selected apps on this Mac".to_string()],
            project_eligible: true,
            supported: cfg!(target_os = "macos"),
            availability_reason_code: (!cfg!(target_os = "macos"))
                .then(|| "unsupported_platform".to_string()),
        },
        ConnectorManifest {
            manifest_id: "google_workspace".to_string(),
            name: "Google Workspace".to_string(),
            version: 1,
            transport: "https_api".to_string(),
            auth_method: "oauth_authorization_code_pkce".to_string(),
            tools: vec![
                tool("gmail.search", "read", "Search and read Gmail."),
                tool("gmail.read", "read", "Read one Gmail message."),
                tool(
                    "gmail.draft",
                    "write",
                    "Create a visible Gmail draft after approval.",
                ),
                tool("calendar.read", "read", "Read Google Calendar."),
                tool(
                    "calendar.create",
                    "write",
                    "Create an event after approval.",
                ),
                tool(
                    "calendar.update",
                    "write",
                    "Change an event after approval.",
                ),
                tool("drive.search", "read", "Search and read Drive files."),
                tool("drive.read", "read", "Read one Drive file."),
                tool(
                    "drive.export",
                    "write",
                    "Export a Drive file after approval.",
                ),
            ],
            requested_permissions: vec![
                "Read Gmail".to_string(),
                "Create Gmail drafts".to_string(),
                "Read and change Calendar after approval".to_string(),
                "Search and read Drive".to_string(),
            ],
            base_scopes: oauth_base_scopes("google_workspace"),
            operation_grants: google_operation_grants(),
            data_destinations: vec![
                "https://accounts.google.com".to_string(),
                "https://oauth2.googleapis.com".to_string(),
                "https://gmail.googleapis.com".to_string(),
                "https://www.googleapis.com".to_string(),
            ],
            project_eligible: true,
            supported: GOOGLE_CLIENT_ID.is_some() && GOOGLE_CLIENT_SECRET.is_some(),
            availability_reason_code: (GOOGLE_CLIENT_ID.is_none()
                || GOOGLE_CLIENT_SECRET.is_none())
            .then(|| "build_missing_oauth_client".to_string()),
        },
        slack_manifest(),
        super::microsoft365::descriptor(MICROSOFT_CLIENT_ID),
        ConnectorManifest {
            manifest_id: "mcp_runtime".to_string(),
            name: "Configured MCP Servers".to_string(),
            version: 1,
            transport: "existing_mcp_runtime".to_string(),
            auth_method: "runtime_defined".to_string(),
            tools: vec![],
            requested_permissions: vec![
                "Permissions declared by each server tool schema".to_string()
            ],
            base_scopes: vec![],
            operation_grants: vec![],
            data_destinations: vec!["Each server's configured and pinned destination".to_string()],
            project_eligible: true,
            supported: true,
            availability_reason_code: None,
        },
    ]
}

pub(super) fn manifest(id: &str) -> Result<ConnectorManifest, String> {
    manifests()
        .into_iter()
        .find(|item| item.manifest_id == id)
        .ok_or_else(|| "Unknown connector manifest.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_desktop_identities_are_compiled_into_the_manifest() {
        let google = manifest("google_workspace").unwrap();
        let google_credential_available = google_oauth_client_secret().is_some();
        assert_eq!(google.supported, google_credential_available);
        assert_eq!(
            google.availability_reason_code.is_none(),
            google_credential_available
        );
        assert_eq!(
            oauth_client_id("google_workspace"),
            Some("778235629301-i9h2mds07jmh85d0v3pdtka2limn9vo1.apps.googleusercontent.com")
        );
        let slack = manifest("slack").unwrap();
        assert!(slack.supported);
        assert!(slack.availability_reason_code.is_none());
        assert_eq!(
            oauth_client_id("slack"),
            Some("11553092442215.11596794910288")
        );
        assert_eq!(option_env!("OOMU_SLACK_OAUTH_REDIRECT_PORT"), Some("53682"));
    }

    #[test]
    fn oauth_manifest_base_scopes_are_the_authoritative_request_sets() {
        for manifest_id in [
            "google_workspace",
            "slack",
            super::super::microsoft365::MANIFEST_ID,
        ] {
            assert_eq!(
                manifest(manifest_id).unwrap().base_scopes,
                oauth_base_scopes(manifest_id)
            );
        }
        assert_eq!(
            super::super::microsoft365::requested_scopes(&[]).unwrap(),
            oauth_base_scopes(super::super::microsoft365::MANIFEST_ID)
        );
    }

    #[test]
    fn slack_read_and_messaging_tiers_are_distinct_and_additive() {
        let read = slack_requested_scopes(&[]).unwrap();
        assert!(!read.iter().any(|scope| scope == "chat:write"));
        let messaging = slack_requested_scopes(&[SLACK_MESSAGING_OPERATION.to_string()]).unwrap();
        assert!(messaging.iter().any(|scope| scope == "chat:write"));
        assert!(read.iter().all(|scope| messaging.contains(scope)));
        let descriptor = manifest("slack").unwrap();
        let grant = descriptor
            .operation_grants
            .iter()
            .find(|grant| grant.operation == SLACK_MESSAGING_OPERATION)
            .expect("Slack exposes one reviewed messaging upgrade");
        assert_eq!(grant.access_level, "write");
        assert!(grant.remote_mutation);
        assert_eq!(grant.available, super::super::oauth_broker::configured());
        assert_eq!(
            grant.unavailable_reason_code.as_deref(),
            (!super::super::oauth_broker::configured()).then_some("build_missing_oauth_broker")
        );
        assert!(grant
            .required_scopes
            .iter()
            .any(|scope| scope == "chat:write"));
    }

    #[test]
    fn google_starts_with_identity_and_adds_only_the_requested_capability() {
        let base = google_requested_scopes(&[]).unwrap();
        assert_eq!(base, vec![GOOGLE_EMAIL_SCOPE, GOOGLE_PROFILE_SCOPE]);

        let gmail = google_requested_scopes(&["gmail.read".to_string()]).unwrap();
        assert!(gmail.iter().any(|scope| scope == GOOGLE_GMAIL_READ));
        assert!(!gmail.iter().any(|scope| scope == GOOGLE_CALENDAR_READ));
        assert!(!gmail.iter().any(|scope| scope == GOOGLE_DRIVE_READ));

        let grants = google_capability_grants(&gmail);
        assert!(grants
            .iter()
            .find(|grant| grant.capability_id == "gmail.read")
            .is_some_and(|grant| grant.granted));
        assert!(grants
            .iter()
            .find(|grant| grant.capability_id == "calendar.read")
            .is_some_and(|grant| !grant.granted));
        assert_eq!(
            google_requested_scopes(&["youtube.upload".to_string()]).unwrap_err(),
            "connector_incremental_consent_unsupported",
        );
    }

    #[test]
    fn google_scope_aliases_are_semantically_equivalent() {
        let legacy_grant = vec![
            "openid".to_string(),
            "email".to_string(),
            "profile".to_string(),
        ];
        let requested = vec![
            GOOGLE_EMAIL_SCOPE.to_string(),
            GOOGLE_PROFILE_SCOPE.to_string(),
        ];
        assert_eq!(
            normalize_google_scopes(&legacy_grant),
            vec![GOOGLE_EMAIL_SCOPE, GOOGLE_PROFILE_SCOPE]
        );
        assert!(google_scopes_include(&requested, &legacy_grant));
    }

    #[test]
    fn google_missing_capability_is_not_granted() {
        assert!(!google_scopes_include(
            &vec![
                "email".to_string(),
                "profile".to_string(),
                GOOGLE_GMAIL_READ.to_string(),
            ],
            &vec![
                "openid".to_string(),
                GOOGLE_EMAIL_SCOPE.to_string(),
                GOOGLE_PROFILE_SCOPE.to_string(),
            ],
        ));
    }

    #[test]
    fn apple_apps_advertises_only_real_bounded_native_access() {
        let apple = manifest("apple_apps").unwrap();
        assert_eq!(apple.transport, "native_macos");
        let photos = apple
            .tools
            .iter()
            .find(|tool| tool.name == "photos.read")
            .expect("native Photos capability is advertised");
        assert_eq!(photos.risk, "read");
        assert!(photos.description.contains("bounded photo metadata"));
        let contacts = apple
            .tools
            .iter()
            .find(|tool| tool.name == "contacts.read")
            .expect("native Contacts capability is advertised");
        assert_eq!(contacts.risk, "read");
        assert!(contacts.description.contains("when asked"));
        assert!(apple
            .requested_permissions
            .iter()
            .any(|permission| permission.contains("Calendar range you ask")));
        assert!(apple
            .requested_permissions
            .iter()
            .any(|permission| permission.contains("contacts you ask")));
        let music = apple
            .tools
            .iter()
            .find(|tool| tool.name == "music.read")
            .expect("native Music capability is advertised");
        assert_eq!(music.risk, "read");
        assert!(music.description.contains("newest-added song metadata"));
        assert!(apple
            .requested_permissions
            .iter()
            .any(|permission| permission.contains("Apple Music when you ask")));
    }
}
