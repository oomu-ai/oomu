use super::{
    contracts::{DesktopActionKind, DesktopObservedElement},
    driver::{DriverElement, ResolvedDriverTarget},
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use rand_core::{OsRng, RngCore};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(crate) struct ReferenceContext<'a> {
    pub session_id: &'a str,
    pub project_id: &'a str,
    pub task_run_id: &'a str,
    pub bundle_id: &'a str,
    pub process_id: u64,
    pub window_id: &'a str,
    pub revision: u64,
    pub generation: u64,
    pub now_ms: i64,
}

#[derive(Clone, Debug)]
struct ReferenceBinding {
    session_id: String,
    project_id: String,
    task_run_id: String,
    bundle_id: String,
    process_id: u64,
    window_id: String,
    revision: u64,
    generation: u64,
    expires_at_ms: i64,
    target: ResolvedDriverTarget,
    visible: bool,
    enabled: bool,
    in_modal: bool,
    supported_actions: Vec<DesktopActionKind>,
}

#[derive(Default)]
pub(crate) struct ReferenceVault {
    bindings: HashMap<String, ReferenceBinding>,
}

impl ReferenceVault {
    pub(crate) fn invalidate_session(&mut self, session_id: &str) {
        self.bindings
            .retain(|_, binding| binding.session_id != session_id);
    }

    pub(crate) fn issue(
        &mut self,
        context: &ReferenceContext<'_>,
        element: DriverElement,
        expires_at_ms: i64,
    ) -> DesktopObservedElement {
        let reference = opaque_id("appref");
        let element_key = element.element_key;
        let target = ResolvedDriverTarget {
            element_key: element_key.clone(),
            role: element.role.clone(),
            secure: element.secure,
            in_modal: element.in_modal,
            geometry: element.geometry,
        };
        self.bindings.insert(
            reference.clone(),
            ReferenceBinding {
                session_id: context.session_id.to_string(),
                project_id: context.project_id.to_string(),
                task_run_id: context.task_run_id.to_string(),
                bundle_id: context.bundle_id.to_string(),
                process_id: context.process_id,
                window_id: context.window_id.to_string(),
                revision: context.revision,
                generation: context.generation,
                expires_at_ms,
                target,
                visible: element.visible,
                enabled: element.enabled,
                in_modal: element.in_modal,
                supported_actions: element.supported_actions.clone(),
            },
        );
        DesktopObservedElement {
            element_key,
            reference,
            role: element.role,
            label: element.label.map(|value| value.chars().take(240).collect()),
            value_digest: (!element.secure).then_some(element.value_digest).flatten(),
            secure: element.secure,
            visible: element.visible,
            enabled: element.enabled,
            in_modal: element.in_modal,
            supported_actions: element.supported_actions,
            geometry: element.geometry,
            expires_at_ms,
        }
    }

    pub(crate) fn resolve(
        &self,
        reference: &str,
        action: DesktopActionKind,
        modal: bool,
        context: &ReferenceContext<'_>,
    ) -> AppControlResult<ResolvedDriverTarget> {
        let binding = self.bindings.get(reference).ok_or_else(stale_reference)?;
        if binding.bundle_id != context.bundle_id
            || binding.process_id != context.process_id
            || binding.window_id != context.window_id
        {
            return Err(AppControlError::new(
                AppControlErrorCode::CrossApplicationReference,
                "The target belongs to a different app, process, or window.",
            ));
        }
        if binding.session_id != context.session_id
            || binding.project_id != context.project_id
            || binding.task_run_id != context.task_run_id
        {
            return Err(AppControlError::new(
                AppControlErrorCode::TaskBindingMismatch,
                "The target is not bound to this Task session.",
            ));
        }
        if binding.revision != context.revision
            || binding.generation != context.generation
            || binding.expires_at_ms < context.now_ms
        {
            return Err(stale_reference());
        }
        if binding.target.secure {
            return Err(AppControlError::new(
                AppControlErrorCode::SecureField,
                "Protected fields require human control.",
            ));
        }
        if !binding.visible || !binding.enabled {
            return Err(AppControlError::new(
                AppControlErrorCode::AmbiguousTarget,
                "The target is not visibly actionable.",
            ));
        }
        if modal && !binding.in_modal {
            return Err(AppControlError::new(
                AppControlErrorCode::AmbiguousTarget,
                "A modal window changed the available target.",
            ));
        }
        if !binding.supported_actions.contains(&action) {
            return Err(AppControlError::new(
                AppControlErrorCode::AmbiguousTarget,
                "The target does not support the requested semantic action.",
            ));
        }
        if action == DesktopActionKind::DragDrop && binding.target.geometry.is_none() {
            return Err(AppControlError::new(
                AppControlErrorCode::AmbiguousTarget,
                "Drag and drop requires fresh visible target bounds.",
            ));
        }
        Ok(binding.target.clone())
    }
}

fn stale_reference() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::StaleReference,
        "The screen changed; a fresh observation is required.",
    )
}

pub(crate) fn opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}_{}", hex::encode(bytes))
}
