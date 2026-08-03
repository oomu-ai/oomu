use super::{
    contracts::{
        DesktopActionKind, DesktopSemanticAction, ExpectedOutcomeKind, QualifiedAppIcon,
        QualifiedAppleEvent, QualifiedMenuCommand,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use crate::db::PersistenceEngine;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const BROWSER_BUNDLES: &[&str] = &[
    "com.apple.Safari",
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "org.mozilla.firefox",
    "com.brave.Browser",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationQualification {
    Qualified,
    ObservationOnly,
    Browser,
}

#[derive(Clone, Debug)]
pub struct DesktopAppProfile {
    pub qualification: ApplicationQualification,
    pub display_name: String,
    pub icon: QualifiedAppIcon,
    pub allowed_actions: HashSet<DesktopActionKind>,
}

fn actions(values: &[DesktopActionKind]) -> HashSet<DesktopActionKind> {
    values.iter().copied().collect()
}

pub fn app_profile(bundle_id: &str, reported_name: &str) -> DesktopAppProfile {
    use DesktopActionKind::*;
    if BROWSER_BUNDLES.contains(&bundle_id) {
        return DesktopAppProfile {
            qualification: ApplicationQualification::Browser,
            display_name: reported_name.to_string(),
            icon: QualifiedAppIcon::Generic,
            allowed_actions: HashSet::new(),
        };
    }
    let (display_name, icon, allowed) = match bundle_id {
        "com.apple.finder" => (
            "Finder",
            QualifiedAppIcon::Finder,
            actions(&[
                Focus, Press, Select, InvokeMenu, Scroll, DragDrop, AppleEvent,
            ]),
        ),
        "com.apple.Preview" => (
            "Preview",
            QualifiedAppIcon::Preview,
            actions(&[
                Focus, Press, Select, InvokeMenu, Scroll, ChooseFile, AppleEvent,
            ]),
        ),
        "com.apple.mail" => (
            "Mail",
            QualifiedAppIcon::Mail,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, AppleEvent,
            ]),
        ),
        "com.apple.iCal" => (
            "Calendar",
            QualifiedAppIcon::Calendar,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, AppleEvent,
            ]),
        ),
        "com.apple.Numbers" => (
            "Numbers",
            QualifiedAppIcon::Numbers,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, ChooseFile, AppleEvent,
            ]),
        ),
        "com.apple.Keynote" => (
            "Keynote",
            QualifiedAppIcon::Keynote,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, ChooseFile, AppleEvent,
            ]),
        ),
        "com.microsoft.Excel" => (
            "Excel",
            QualifiedAppIcon::Excel,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, ChooseFile, AppleEvent,
            ]),
        ),
        "com.microsoft.Powerpoint" => (
            "PowerPoint",
            QualifiedAppIcon::Powerpoint,
            actions(&[
                Focus, Press, Select, TypeText, InvokeMenu, Scroll, ChooseFile, AppleEvent,
            ]),
        ),
        _ => {
            return DesktopAppProfile {
                qualification: ApplicationQualification::ObservationOnly,
                display_name: reported_name.chars().take(120).collect(),
                icon: QualifiedAppIcon::Generic,
                allowed_actions: HashSet::new(),
            }
        }
    };
    DesktopAppProfile {
        qualification: ApplicationQualification::Qualified,
        display_name: display_name.to_string(),
        icon,
        allowed_actions: allowed,
    }
}

pub fn validate_typed_adapter(
    bundle_id: &str,
    action: &DesktopSemanticAction,
) -> AppControlResult<()> {
    let profile = app_profile(bundle_id, bundle_id);
    if profile.qualification == ApplicationQualification::Browser {
        return Err(AppControlError::new(
            AppControlErrorCode::BrowserRouteRequired,
            "Browser work must use the guarded browser runtime.",
        ));
    }
    if profile.qualification != ApplicationQualification::Qualified {
        return Err(AppControlError::new(
            AppControlErrorCode::ObservationOnlyApplication,
            "This application is observation-only until a reviewed adapter is available.",
        ));
    }
    if !profile.allowed_actions.contains(&action.kind()) {
        return Err(AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "That semantic action is not qualified for this application.",
        ));
    }
    if matches!(
        action,
        DesktopSemanticAction::InvokeMenu {
            command: QualifiedMenuCommand::Export
        }
    ) {
        return Err(AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "Export is observation-only until an app-specific adapter is installed.",
        ));
    }
    if matches!(action, DesktopSemanticAction::DragDrop { .. }) && bundle_id != "com.apple.finder" {
        return Err(AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "Drag and drop is qualified only for Finder.",
        ));
    }
    if matches!(action, DesktopSemanticAction::ChooseFile { .. })
        && !matches!(
            bundle_id,
            "com.apple.Preview"
                | "com.apple.Numbers"
                | "com.apple.Keynote"
                | "com.microsoft.Excel"
                | "com.microsoft.Powerpoint"
        )
    {
        return Err(AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "File selection is not qualified for this application.",
        ));
    }
    if let DesktopSemanticAction::AppleEvent { command } = action {
        if *command != QualifiedAppleEvent::ActivateApplication {
            return Err(AppControlError::new(
                AppControlErrorCode::InvalidRequest,
                "That typed application command is not qualified for this application.",
            ));
        }
    }
    Ok(())
}

pub fn validate_expected_outcome(
    action: &DesktopSemanticAction,
    expected: ExpectedOutcomeKind,
) -> AppControlResult<()> {
    let qualified = match (action, expected) {
        (DesktopSemanticAction::Focus { .. }, ExpectedOutcomeKind::ElementState) => true,
        (
            DesktopSemanticAction::Select { .. } | DesktopSemanticAction::TypeText { .. },
            ExpectedOutcomeKind::ElementValue,
        ) => true,
        (
            DesktopSemanticAction::Press { .. },
            ExpectedOutcomeKind::ElementState
            | ExpectedOutcomeKind::WindowState
            | ExpectedOutcomeKind::ApplicationState,
        ) => true,
        (
            DesktopSemanticAction::InvokeMenu { .. },
            ExpectedOutcomeKind::WindowState | ExpectedOutcomeKind::ApplicationState,
        ) => true,
        (DesktopSemanticAction::Scroll { .. }, ExpectedOutcomeKind::ApplicationState) => true,
        (DesktopSemanticAction::DragDrop { .. }, ExpectedOutcomeKind::ApplicationState) => true,
        (DesktopSemanticAction::ChooseFile { .. }, ExpectedOutcomeKind::WindowState) => true,
        (DesktopSemanticAction::AppleEvent { .. }, ExpectedOutcomeKind::ApplicationState) => true,
        _ => false,
    };
    if qualified {
        Ok(())
    } else {
        Err(AppControlError::new(
            AppControlErrorCode::InvalidRequest,
            "The requested result does not match the qualified app action.",
        ))
    }
}

#[derive(Clone, Debug)]
pub struct AuthorityRequest {
    pub project_id: String,
    pub task_run_id: String,
    pub session_id: String,
    pub bundle_id: String,
    pub action_kind: DesktopActionKind,
    pub action_arguments_hash: String,
    pub will_change_data: bool,
}

#[derive(Clone, Debug)]
pub struct AuthorityDecision {
    pub authorized: bool,
    pub decision_id: String,
}

pub trait DesktopAuthorityEvaluator: Send + Sync {
    fn evaluate(&self, request: &AuthorityRequest) -> AppControlResult<AuthorityDecision>;

    fn register_direct_approval(&self, _request: &AuthorityRequest) -> AppControlResult<()> {
        Err(AppControlError::new(
            AppControlErrorCode::Unauthorized,
            "The configured authority cannot register a direct approval.",
        ))
    }
}

#[derive(Default)]
pub struct DenyAllDesktopAuthority;

impl DesktopAuthorityEvaluator for DenyAllDesktopAuthority {
    fn evaluate(&self, _request: &AuthorityRequest) -> AppControlResult<AuthorityDecision> {
        Err(AppControlError::new(
            AppControlErrorCode::Unauthorized,
            "App control requires a Task-bound authority decision.",
        ))
    }
}

/// Production authority adapter for the reviewed, expiring approval scopes
/// introduced by the trust UX. Each use is Project- and Task-bound, consumes
/// one use, and binds the exact serialized action arguments by digest.
#[derive(Clone)]
pub struct ReviewedScopeDesktopAuthority {
    engine: PersistenceEngine,
    direct_approvals: Arc<Mutex<HashSet<String>>>,
}

impl ReviewedScopeDesktopAuthority {
    pub fn new(engine: PersistenceEngine) -> Self {
        Self {
            engine,
            direct_approvals: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    pub fn approval_binding(request: &AuthorityRequest) -> DesktopApprovalBinding {
        DesktopApprovalBinding {
            principal: "local_principal".to_string(),
            action_class: "app_control".to_string(),
            canonical_resource: format!("{}/{:?}", request.bundle_id, request.action_kind)
                .to_ascii_lowercase(),
            argument_class: format!("exact:{}", request.action_arguments_hash),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesktopApprovalBinding {
    pub principal: String,
    pub action_class: String,
    pub canonical_resource: String,
    pub argument_class: String,
}

impl DesktopAuthorityEvaluator for ReviewedScopeDesktopAuthority {
    fn evaluate(&self, request: &AuthorityRequest) -> AppControlResult<AuthorityDecision> {
        let binding = Self::approval_binding(request);
        let key = direct_approval_key(request);
        let direct = self
            .direct_approvals
            .lock()
            .map_err(|_| {
                AppControlError::new(
                    AppControlErrorCode::Unauthorized,
                    "The direct approval registry is unavailable.",
                )
            })?
            .remove(&key);
        let authorized = if direct {
            true
        } else {
            crate::approval_scopes::authorize(
                &self.engine,
                &binding.principal,
                Some(&request.project_id),
                Some(&request.task_run_id),
                &binding.action_class,
                &binding.canonical_resource,
                &binding.argument_class,
                1,
            )
            .map_err(|_| {
                AppControlError::new(
                    AppControlErrorCode::Unauthorized,
                    "App control could not verify the reviewed Task approval.",
                )
            })?
        };
        let digest = Sha256::digest(
            format!(
                "{}:{}:{}:{}",
                request.task_run_id, binding.canonical_resource, binding.argument_class, authorized
            )
            .as_bytes(),
        );
        Ok(AuthorityDecision {
            authorized,
            decision_id: format!("appauthority_{}", &hex::encode(digest)[..32]),
        })
    }

    fn register_direct_approval(&self, request: &AuthorityRequest) -> AppControlResult<()> {
        self.direct_approvals
            .lock()
            .map_err(|_| {
                AppControlError::new(
                    AppControlErrorCode::Unauthorized,
                    "The direct approval registry is unavailable.",
                )
            })?
            .insert(direct_approval_key(request));
        Ok(())
    }
}

fn direct_approval_key(request: &AuthorityRequest) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        request.project_id,
        request.task_run_id,
        request.session_id,
        request.bundle_id,
        request.action_arguments_hash
    )
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ScopedFileRoots {
    roots: Vec<PathBuf>,
    granted_files: HashMap<String, PathBuf>,
}

impl ScopedFileRoots {
    pub(crate) fn new(roots: Vec<PathBuf>) -> AppControlResult<Self> {
        if roots.len() > 16 {
            return Err(AppControlError::new(
                AppControlErrorCode::FileScopeViolation,
                "Too many file roots were requested.",
            ));
        }
        let mut canonical = Vec::with_capacity(roots.len());
        for root in roots {
            let path = fs::canonicalize(root).map_err(|_| {
                AppControlError::new(
                    AppControlErrorCode::FileScopeViolation,
                    "An approved file root is unavailable.",
                )
            })?;
            if !path.is_dir() || path.parent().is_none() {
                return Err(AppControlError::new(
                    AppControlErrorCode::FileScopeViolation,
                    "File roots must be specific existing folders.",
                ));
            }
            if !canonical.contains(&path) {
                canonical.push(path);
            }
        }
        Ok(Self {
            roots: canonical,
            granted_files: HashMap::new(),
        })
    }

    pub(crate) fn add_granted_file(
        &mut self,
        grant_id: String,
        selected_file: PathBuf,
    ) -> AppControlResult<()> {
        if self.granted_files.len() >= 16 || !valid_file_grant_id(&grant_id) {
            return Err(file_scope_error("The selected file grant is invalid."));
        }
        let canonical = fs::canonicalize(selected_file)
            .map_err(|_| file_scope_error("The selected file is unavailable."))?;
        if !canonical.is_file() {
            return Err(file_scope_error("The selected item must be a file."));
        }
        canonical
            .parent()
            .filter(|parent| parent.parent().is_some())
            .ok_or_else(|| file_scope_error("The selected file folder is too broad."))?;
        self.granted_files.insert(grant_id, canonical);
        Ok(())
    }

    pub(crate) fn canonical_granted_file(&self, grant_id: &str) -> AppControlResult<PathBuf> {
        let selected = self
            .granted_files
            .get(grant_id)
            .ok_or_else(|| file_scope_error("The selected file grant is unavailable."))?;
        let canonical = fs::canonicalize(selected).map_err(|_| {
            AppControlError::new(
                AppControlErrorCode::FileScopeViolation,
                "The selected file is unavailable.",
            )
        })?;
        if &canonical == selected && canonical.is_file() {
            Ok(canonical)
        } else {
            Err(AppControlError::new(
                AppControlErrorCode::FileScopeViolation,
                "The selected file is outside the Task's approved folders.",
            ))
        }
    }

    pub(crate) fn canonical_file(&self, path: &Path) -> AppControlResult<PathBuf> {
        let canonical = fs::canonicalize(path)
            .map_err(|_| file_scope_error("The selected file is unavailable."))?;
        if self.roots.iter().any(|root| canonical.starts_with(root)) && canonical.is_file() {
            Ok(canonical)
        } else {
            Err(file_scope_error(
                "The selected file is outside the Task's approved folders.",
            ))
        }
    }

    pub(crate) fn file_name(&self, grant_id: &str) -> Option<String> {
        self.granted_files
            .get(grant_id)
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().chars().take(120).collect())
    }
}

pub(crate) fn valid_file_grant_id(value: &str) -> bool {
    value.strip_prefix("appfile_").is_some_and(|suffix| {
        suffix.len() == 48
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn file_scope_error(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::FileScopeViolation, message)
}
