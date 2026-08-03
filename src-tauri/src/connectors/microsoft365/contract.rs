use super::super::{ConnectorCapabilityGrant, ConnectorOperationGrant};
use std::collections::BTreeSet;

pub(in crate::connectors) const MANIFEST_ID: &str = "microsoft_365";
pub(in crate::connectors) const AUTHORIZATION_ENDPOINT: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
pub(super) const COMMON_TOKEN_ENDPOINT: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/token";
pub(super) const GRAPH_ORIGIN: &str = "https://graph.microsoft.com";
pub(super) const GRAPH_ROOT: &str = "https://graph.microsoft.com/v1.0";
pub(in crate::connectors) const LOOPBACK_REDIRECT_PORT: u16 = 53_683;

pub(in crate::connectors) const OUTLOOK_MAIL_SEARCH: &str = "outlook.mail.search";
pub(in crate::connectors) const OUTLOOK_MAIL_READ: &str = "outlook.mail.read";
pub(in crate::connectors) const OUTLOOK_MAIL_DRAFT: &str = "outlook.mail.draft";
pub(super) const OUTLOOK_CALENDAR_READ: &str = "outlook.calendar.read";
pub(super) const OUTLOOK_CALENDAR_DRAFT: &str = "outlook.calendar.draft_event";
pub(super) const ONEDRIVE_SEARCH: &str = "onedrive.file.search";
pub(super) const ONEDRIVE_READ: &str = "onedrive.file.read";
pub(super) const ONEDRIVE_WRITE: &str = "onedrive.file.write";
pub(super) const SHAREPOINT_SEARCH: &str = "sharepoint.file.search";
pub(super) const SHAREPOINT_READ: &str = "sharepoint.file.read";
pub(super) const SHAREPOINT_WRITE: &str = "sharepoint.file.write";
pub(super) const SHAREPOINT_RESOLVE: &str = "sharepoint.site.resolve";
pub(super) const TEAMS_LIST: &str = "teams.chat.list";
pub(super) const TEAMS_SEARCH: &str = "teams.chat.search";
pub(in crate::connectors) const TEAMS_DRAFT: &str = "teams.chat.draft_message";

const BASE_SCOPES: &[&str] = &["openid", "profile", "email", "offline_access", "User.Read"];

#[derive(Clone, Debug)]
pub(super) struct OperationGrant {
    pub operation: &'static str,
    pub access_level: &'static str,
    pub scopes: &'static [&'static str],
    pub admin_consent_required: bool,
    pub remote_mutation: bool,
}

const OPERATION_GRANTS: &[OperationGrant] = &[
    OperationGrant {
        operation: OUTLOOK_MAIL_SEARCH,
        access_level: "read",
        scopes: &["Mail.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: OUTLOOK_MAIL_READ,
        access_level: "read",
        scopes: &["Mail.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: OUTLOOK_MAIL_DRAFT,
        access_level: "draft_write",
        scopes: &["Mail.ReadWrite"],
        admin_consent_required: false,
        remote_mutation: true,
    },
    OperationGrant {
        operation: OUTLOOK_CALENDAR_READ,
        access_level: "read",
        scopes: &["Calendars.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: OUTLOOK_CALENDAR_DRAFT,
        access_level: "local_draft",
        scopes: &[],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: ONEDRIVE_SEARCH,
        access_level: "read",
        scopes: &["Files.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: ONEDRIVE_READ,
        access_level: "read",
        scopes: &["Files.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: ONEDRIVE_WRITE,
        access_level: "write",
        scopes: &["Files.ReadWrite"],
        admin_consent_required: false,
        remote_mutation: true,
    },
    OperationGrant {
        operation: SHAREPOINT_SEARCH,
        access_level: "tenant_read",
        scopes: &["Sites.Read.All"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: SHAREPOINT_READ,
        access_level: "tenant_read",
        scopes: &["Sites.Read.All"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: SHAREPOINT_WRITE,
        access_level: "tenant_write",
        scopes: &["Sites.ReadWrite.All"],
        admin_consent_required: false,
        remote_mutation: true,
    },
    OperationGrant {
        operation: TEAMS_SEARCH,
        access_level: "read",
        scopes: &["Chat.Read"],
        admin_consent_required: false,
        remote_mutation: false,
    },
    OperationGrant {
        operation: TEAMS_DRAFT,
        access_level: "local_draft",
        scopes: &[],
        admin_consent_required: false,
        remote_mutation: false,
    },
];

pub(super) fn operation_grant(operation: &str) -> Result<&'static OperationGrant, String> {
    OPERATION_GRANTS
        .iter()
        .find(|grant| grant.operation == operation)
        .ok_or_else(|| "microsoft_operation_unsupported".to_string())
}

pub(in crate::connectors) fn base_scopes() -> Vec<String> {
    BASE_SCOPES
        .iter()
        .map(|scope| (*scope).to_string())
        .collect()
}

pub(super) fn manifest_operation_grants() -> Vec<ConnectorOperationGrant> {
    OPERATION_GRANTS
        .iter()
        .map(|grant| ConnectorOperationGrant {
            operation: grant.operation.to_string(),
            purpose_code: format!(
                "connector.microsoft365.consent.{}",
                grant.operation.replace('.', "_")
            ),
            required_scopes: grant
                .scopes
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            access_level: grant.access_level.to_string(),
            remote_mutation: grant.remote_mutation,
            admin_consent_required: grant.admin_consent_required,
            available: true,
            unavailable_reason_code: None,
        })
        .collect()
}

pub(in crate::connectors) fn requested_scopes(
    operations: &[String],
) -> Result<Vec<String>, String> {
    let mut requested = base_scopes();
    for operation in operations {
        for scope in operation_grant(operation.trim())?.scopes {
            if !requested
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(scope))
            {
                requested.push((*scope).to_string());
            }
        }
    }
    Ok(requested)
}

pub(in crate::connectors) fn merge_scopes(
    requested: &[String],
    existing: &[String],
) -> Vec<String> {
    requested
        .iter()
        .chain(existing)
        .map(|scope| scope.trim())
        .filter(|scope| !scope.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn has_scopes(granted: &[String], required: &[&str]) -> bool {
    required.iter().all(|required_scope| {
        granted
            .iter()
            .any(|scope| scope.eq_ignore_ascii_case(required_scope))
    })
}

pub(super) fn require_operation_scopes(granted: &[String], operation: &str) -> Result<(), String> {
    let required: &[&str] = match operation {
        SHAREPOINT_RESOLVE => &["Sites.Read.All"],
        TEAMS_LIST => &["Chat.Read"],
        _ => operation_grant(operation)?.scopes,
    };
    if has_scopes(granted, required) {
        Ok(())
    } else {
        Err("microsoft_incremental_consent_required".to_string())
    }
}

pub(super) fn capability_grants(
    scopes: &[String],
    account_kind: Option<&str>,
) -> Vec<ConnectorCapabilityGrant> {
    OPERATION_GRANTS
        .iter()
        .map(|grant| {
            let work_only =
                grant.operation.starts_with("sharepoint.") || grant.operation.starts_with("teams.");
            let available = !(work_only && account_kind == Some("personal"));
            ConnectorCapabilityGrant {
                capability_id: grant.operation.to_string(),
                access_level: grant.access_level.to_string(),
                required_scopes: grant
                    .scopes
                    .iter()
                    .map(|scope| (*scope).to_string())
                    .collect(),
                granted: available && has_scopes(scopes, grant.scopes),
                admin_consent_required: grant.admin_consent_required,
                remote_mutation: grant.remote_mutation,
                available,
                unavailable_reason_code: (!available)
                    .then(|| "microsoft_capability_work_account_required".to_string()),
            }
        })
        .collect()
}

pub(super) fn data_routing() -> Vec<String> {
    vec![
        "https://login.microsoftonline.com".to_string(),
        GRAPH_ORIGIN.to_string(),
        "https://*.sharepoint.com".to_string(),
        "https://*.sharepointonline.com".to_string(),
        "https://*.1drv.com".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_consent_requests_only_the_selected_capability_scopes() {
        let mail = requested_scopes(&[OUTLOOK_MAIL_SEARCH.to_string()]).unwrap();
        assert!(mail.iter().any(|scope| scope == "Mail.Read"));
        assert!(!mail.iter().any(|scope| scope == "Mail.ReadWrite"));
        assert!(!mail.iter().any(|scope| scope == "Files.Read"));

        let draft = requested_scopes(&[OUTLOOK_MAIL_DRAFT.to_string()]).unwrap();
        assert!(draft.iter().any(|scope| scope == "Mail.ReadWrite"));
        assert!(!draft.iter().any(|scope| scope == "Mail.Send"));
    }

    #[test]
    fn local_drafts_do_not_expand_remote_grants() {
        let scopes =
            requested_scopes(&[OUTLOOK_CALENDAR_DRAFT.to_string(), TEAMS_DRAFT.to_string()])
                .unwrap();
        assert_eq!(scopes.len(), BASE_SCOPES.len());
        assert!(operation_grant("outlook.mail.send").is_err());
        assert!(operation_grant("sharepoint.share").is_err());
    }

    #[test]
    fn later_consent_preserves_every_earlier_capability_scope() {
        let earlier = requested_scopes(&[OUTLOOK_MAIL_SEARCH.to_string()]).unwrap();
        let later = requested_scopes(&[ONEDRIVE_SEARCH.to_string()]).unwrap();
        let merged = merge_scopes(&later, &earlier);
        assert!(has_scopes(&merged, &["Mail.Read"]));
        assert!(has_scopes(&merged, &["Files.Read"]));
        require_operation_scopes(&merged, OUTLOOK_MAIL_SEARCH).unwrap();
        require_operation_scopes(&merged, ONEDRIVE_SEARCH).unwrap();
    }

    #[test]
    fn personal_accounts_do_not_advertise_work_tenant_capabilities() {
        let scopes = vec![
            "Sites.Read.All".to_string(),
            "Chat.Read".to_string(),
            "Mail.Read".to_string(),
        ];
        let personal = capability_grants(&scopes, Some("personal"));
        let work = capability_grants(&scopes, Some("work"));
        let personal_sharepoint = personal
            .iter()
            .find(|grant| grant.capability_id == SHAREPOINT_SEARCH)
            .unwrap();
        let personal_teams = personal
            .iter()
            .find(|grant| grant.capability_id == TEAMS_SEARCH)
            .unwrap();
        assert!(!personal_sharepoint.available && !personal_sharepoint.granted);
        assert!(!personal_teams.available && !personal_teams.granted);
        assert_eq!(
            personal_sharepoint.unavailable_reason_code.as_deref(),
            Some("microsoft_capability_work_account_required")
        );
        assert!(
            work.iter()
                .find(|grant| grant.capability_id == SHAREPOINT_SEARCH)
                .unwrap()
                .available
        );
        assert!(
            personal
                .iter()
                .find(|grant| grant.capability_id == OUTLOOK_MAIL_READ)
                .unwrap()
                .available
        );
    }
}
