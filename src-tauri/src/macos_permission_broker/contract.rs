use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRequestMethod {
    NativePrompt,
    ContextualPrompt,
    SystemSettings,
    DelegatedHelper,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionRefreshAction {
    Recheck,
    ResetEventStoreAndRefresh,
    ReactivateApp,
    RelaunchOwner,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionLifecycleContract {
    pub request_method: PermissionRequestMethod,
    pub after_grant: PermissionRefreshAction,
    pub resume_saved_turn: bool,
    pub read_reuses_grant: bool,
    pub mutations_require_approval: bool,
}

impl PermissionLifecycleContract {
    const fn new(
        request_method: PermissionRequestMethod,
        after_grant: PermissionRefreshAction,
    ) -> Self {
        Self {
            request_method,
            after_grant,
            resume_saved_turn: true,
            read_reuses_grant: true,
            mutations_require_approval: true,
        }
    }
}

pub(super) fn lifecycle(capability: &str) -> PermissionLifecycleContract {
    use PermissionRefreshAction as Refresh;
    use PermissionRequestMethod as Request;

    match capability {
        "calendar" | "reminders" => PermissionLifecycleContract::new(
            Request::NativePrompt,
            Refresh::ResetEventStoreAndRefresh,
        ),
        "contacts" | "photos" | "music" | "camera" | "notifications" => {
            PermissionLifecycleContract::new(Request::NativePrompt, Refresh::Recheck)
        }
        "microphone" | "speech_recognition" => {
            PermissionLifecycleContract::new(Request::DelegatedHelper, Refresh::RelaunchOwner)
        }
        "accessibility" | "screen_control" => {
            PermissionLifecycleContract::new(Request::SystemSettings, Refresh::ReactivateApp)
        }
        "screen_capture" => {
            PermissionLifecycleContract::new(Request::NativePrompt, Refresh::ReactivateApp)
        }
        "mail" | "notes" | "messages" | "finder" | "system_events" => {
            PermissionLifecycleContract::new(Request::ContextualPrompt, Refresh::Recheck)
        }
        "files_and_folders" => {
            PermissionLifecycleContract::new(Request::ContextualPrompt, Refresh::Recheck)
        }
        "local_network" => PermissionLifecycleContract::new(Request::Unsupported, Refresh::None),
        "full_disk_access" => {
            PermissionLifecycleContract::new(Request::SystemSettings, Refresh::RelaunchOwner)
        }
        _ => PermissionLifecycleContract::new(Request::Unsupported, Refresh::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eventkit_capabilities_have_their_own_reset_contract() {
        for capability in ["calendar", "reminders"] {
            let contract = lifecycle(capability);
            assert_eq!(
                contract.after_grant,
                PermissionRefreshAction::ResetEventStoreAndRefresh
            );
        }
        assert_eq!(
            lifecycle("contacts").after_grant,
            PermissionRefreshAction::Recheck
        );
    }

    #[test]
    fn read_grants_never_authorize_mutation() {
        for capability in [
            "calendar",
            "reminders",
            "mail",
            "notes",
            "messages",
            "contacts",
            "photos",
            "music",
            "finder",
            "screen_control",
        ] {
            let contract = lifecycle(capability);
            assert!(contract.read_reuses_grant, "{capability}");
            assert!(contract.mutations_require_approval, "{capability}");
        }
    }

    #[test]
    fn unsupported_capabilities_make_no_request_claim() {
        let contract = lifecycle("not_a_real_capability");
        assert_eq!(
            contract.request_method,
            PermissionRequestMethod::Unsupported
        );
        assert_eq!(contract.after_grant, PermissionRefreshAction::None);
        assert_eq!(
            lifecycle("local_network").request_method,
            PermissionRequestMethod::Unsupported
        );
    }
}
