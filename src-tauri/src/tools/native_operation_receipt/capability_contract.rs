use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppleCapability {
    Calendar,
    Reminders,
    Mail,
    Notes,
    Messages,
    Contacts,
    Photos,
    Music,
    Finder,
    SystemEvents,
    Accessibility,
    ScreenControl,
    ScreenCapture,
    Microphone,
    SpeechRecognition,
    Camera,
    Notifications,
    LocalNetwork,
    FilesAndFolders,
    FullDiskAccess,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeActionClass {
    Read,
    Write,
    Draft,
    Send,
    Delete,
    Control,
    Capture,
    Observe,
    Notify,
    Probe,
}

impl NativeActionClass {
    const ALL: [Self; 10] = [
        Self::Read,
        Self::Write,
        Self::Draft,
        Self::Send,
        Self::Delete,
        Self::Control,
        Self::Capture,
        Self::Observe,
        Self::Notify,
        Self::Probe,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionApprovalClass {
    NativePermissionOnly,
    ResourceScope,
    ExplicitAction,
    ExplicitHighImpact,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeOperationOutcome {
    Succeeded,
    Failed,
    Unmet,
    Unsupported,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CapabilityDescriptor {
    pub id: &'static str,
    pub authority_owner: &'static str,
    pub framework: &'static str,
}

impl AppleCapability {
    pub(crate) const ALL: [Self; 20] = [
        Self::Calendar,
        Self::Reminders,
        Self::Mail,
        Self::Notes,
        Self::Messages,
        Self::Contacts,
        Self::Photos,
        Self::Music,
        Self::Finder,
        Self::SystemEvents,
        Self::Accessibility,
        Self::ScreenControl,
        Self::ScreenCapture,
        Self::Microphone,
        Self::SpeechRecognition,
        Self::Camera,
        Self::Notifications,
        Self::LocalNetwork,
        Self::FilesAndFolders,
        Self::FullDiskAccess,
    ];

    pub(crate) fn descriptor(self) -> CapabilityDescriptor {
        match self {
            Self::Calendar => descriptor("calendar", "main_app", "EventKit"),
            Self::Reminders => descriptor("reminders", "main_app", "EventKit"),
            Self::Mail => descriptor("mail", "main_app", "AppleEvents"),
            Self::Notes => descriptor("notes", "main_app", "AppleEvents"),
            Self::Messages => descriptor("messages", "main_app", "AppleEvents"),
            Self::Contacts => descriptor("contacts", "main_app", "Contacts"),
            Self::Photos => descriptor("photos", "main_app", "PhotoKit"),
            Self::Music => descriptor("music", "main_app", "MediaPlayer"),
            Self::Finder => descriptor("finder", "main_app", "AppleEvents"),
            Self::SystemEvents => descriptor("system_events", "main_app", "AppleEvents"),
            Self::Accessibility | Self::ScreenControl => descriptor(
                if self == Self::Accessibility {
                    "accessibility"
                } else {
                    "screen_control"
                },
                "main_app",
                "ApplicationServices",
            ),
            Self::ScreenCapture => descriptor("screen_capture", "main_app", "CoreGraphics"),
            Self::Microphone => descriptor("microphone", "oomu-speech-bridge", "AVFoundation"),
            Self::SpeechRecognition => {
                descriptor("speech_recognition", "oomu-speech-bridge", "Speech")
            }
            Self::Camera => descriptor("camera", "main_app", "AVFoundation"),
            Self::Notifications => descriptor("notifications", "main_app", "UserNotifications"),
            Self::LocalNetwork => descriptor("local_network", "main_app", "Network"),
            Self::FilesAndFolders => descriptor("files_and_folders", "main_app", "Powerbox"),
            Self::FullDiskAccess => descriptor("full_disk_access", "main_app", "macOS"),
        }
    }

    pub(super) fn supports(self, action: NativeActionClass) -> bool {
        use AppleCapability as Capability;
        use NativeActionClass as Action;
        matches!(
            (self, action),
            (Capability::Calendar, Action::Read | Action::Write)
                | (Capability::Reminders, Action::Read | Action::Write)
                | (
                    Capability::Mail,
                    Action::Read | Action::Draft | Action::Send
                )
                | (Capability::Notes, Action::Read | Action::Write)
                | (Capability::Messages, Action::Read | Action::Draft)
                | (
                    Capability::Contacts | Capability::Photos | Capability::Music,
                    Action::Read
                )
                | (
                    Capability::Finder | Capability::FilesAndFolders,
                    Action::Read | Action::Write | Action::Delete
                )
                | (Capability::SystemEvents, Action::Read | Action::Control)
                | (Capability::Accessibility, Action::Observe | Action::Control)
                | (Capability::ScreenControl, Action::Control)
                | (
                    Capability::ScreenCapture | Capability::Camera,
                    Action::Capture
                )
                | (
                    Capability::Microphone | Capability::SpeechRecognition,
                    Action::Observe
                )
                | (Capability::Notifications, Action::Notify)
                | (Capability::LocalNetwork, Action::Probe)
                | (Capability::FullDiskAccess, Action::Read)
        )
    }
}

const fn descriptor(
    id: &'static str,
    authority_owner: &'static str,
    framework: &'static str,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id,
        authority_owner,
        framework,
    }
}

pub(crate) fn contract_is_complete() -> bool {
    AppleCapability::ALL.iter().all(|capability| {
        let descriptor = capability.descriptor();
        !descriptor.id.is_empty()
            && !descriptor.authority_owner.is_empty()
            && !descriptor.framework.is_empty()
            && NativeActionClass::ALL
                .iter()
                .any(|action| capability.supports(*action))
    })
}
