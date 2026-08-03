//! Native, bounded macOS Accessibility adapter.
//!
//! This module intentionally exposes no AppleScript, shell, JavaScript, raw
//! coordinate, or arbitrary event interface. The only mutation entrypoint is
//! the closed `DesktopSemanticAction` enum.

use super::{
    contracts::{
        AccessibilityPermission, DesktopActionKind, DesktopApplicationObservation,
        DesktopSemanticAction, DesktopWindowObservation, ExpectedOutcomeKind,
    },
    driver::{
        DesktopDriver, DriverActionRequest, DriverActionResult, DriverElement, DriverObservation,
        DriverObservationRequest,
    },
    error::{AppControlError, AppControlErrorCode, AppControlResult},
    references::opaque_id,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::{c_char, c_long, c_void},
    ptr,
    sync::Mutex,
};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFIndex = c_long;
type CFTypeId = usize;
type AXUIElementRef = *const c_void;
type AXError = i32;

const AX_SUCCESS: AXError = 0;
const UTF8_ENCODING: u32 = 0x0800_0100;
const MAX_TREE_DEPTH: usize = 14;
const MAX_NATIVE_ELEMENTS: usize = 2_000;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementGetTypeID() -> CFTypeId;
    fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> AXError;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyActionNames(element: AXUIElementRef, names: *mut CFArrayRef) -> AXError;
    fn AXUIElementIsAttributeSettable(
        element: AXUIElementRef,
        attribute: CFStringRef,
        settable: *mut bool,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFBooleanTrue: CFTypeRef;
    fn CFRetain(value: CFTypeRef) -> CFTypeRef;
    fn CFRelease(value: CFTypeRef);
    fn CFGetTypeID(value: CFTypeRef) -> CFTypeId;
    fn CFStringGetTypeID() -> CFTypeId;
    fn CFStringCreateWithCString(
        allocator: CFAllocatorRef,
        value: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetLength(value: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        value: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFBooleanGetTypeID() -> CFTypeId;
    fn CFBooleanGetValue(value: CFTypeRef) -> bool;
    fn CFArrayGetTypeID() -> CFTypeId;
    fn CFArrayGetCount(value: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(value: CFArrayRef, index: CFIndex) -> *const c_void;
}

struct OwnedCf(CFTypeRef);

impl OwnedCf {
    fn from_created(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }

    fn as_ptr(&self) -> CFTypeRef {
        self.0
    }
}

impl Clone for OwnedCf {
    fn clone(&self) -> Self {
        // SAFETY: `self.0` is a live CoreFoundation object retained by `self`.
        Self(unsafe { CFRetain(self.0) })
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: every `OwnedCf` owns exactly one create/copy/retain reference.
        unsafe { CFRelease(self.0) };
    }
}

// AX objects are process-local handles documented for use across threads.
unsafe impl Send for OwnedCf {}
unsafe impl Sync for OwnedCf {}

#[derive(Clone)]
struct AxElement(OwnedCf);

impl AxElement {
    fn from_owned(value: OwnedCf) -> AppControlResult<Self> {
        // SAFETY: type IDs are pure CoreFoundation queries on a retained object.
        if unsafe { CFGetTypeID(value.as_ptr()) } == unsafe { AXUIElementGetTypeID() } {
            Ok(Self(value))
        } else {
            Err(driver_error(
                "Accessibility returned an unexpected object type.",
            ))
        }
    }

    fn as_ptr(&self) -> AXUIElementRef {
        self.0.as_ptr()
    }

    fn key(&self) -> String {
        format!("ax_{:x}", self.as_ptr() as usize)
    }
}

#[derive(Clone)]
struct RegisteredElement {
    session_id: String,
    element: AxElement,
}

#[derive(Default)]
pub struct MacOsAccessibilityDriver {
    registry: Mutex<HashMap<String, RegisteredElement>>,
}

impl DesktopDriver for MacOsAccessibilityDriver {
    fn observe(&self, request: &DriverObservationRequest) -> AppControlResult<DriverObservation> {
        if !trusted() {
            return Err(AppControlError::new(
                AppControlErrorCode::AccessibilityPermissionMissing,
                "Allow OOMU in Privacy & Security, Accessibility to control apps.",
            ));
        }
        let context = focused_context()?;
        let focused = copy_element(&context.application, "AXFocusedUIElement")
            .ok()
            .map(|element| element.key());
        let modal = copy_bool(&context.window, "AXModal").unwrap_or(false);
        let minimized = copy_bool(&context.window, "AXMinimized").unwrap_or(false);
        let mut collector = TreeCollector::new(request.session_id.clone(), focused);
        collector.walk(&context.window, modal, 0)?;
        let focused_element_key = collector
            .focused_key
            .filter(|key| collector.registry.contains_key(key));

        let mut registry = self
            .registry
            .lock()
            .map_err(|_| driver_error("The Accessibility target registry is unavailable."))?;
        registry.retain(|_, entry| entry.session_id != request.session_id);
        registry.extend(collector.registry);

        Ok(DriverObservation {
            permission: AccessibilityPermission::Granted,
            application: DesktopApplicationObservation {
                bundle_id: context.bundle_id,
                display_name: context.display_name,
                process_id: context.pid as u64,
            },
            window: DesktopWindowObservation {
                window_id: context.window.key(),
                title: copy_string(&context.window, "AXTitle").unwrap_or_default(),
                visible: !minimized,
                modal,
            },
            focused_element_key,
            elements: collector.elements,
            screenshot: None,
        })
    }

    fn perform(&self, request: &DriverActionRequest) -> AppControlResult<DriverActionResult> {
        if !trusted() {
            return Err(AppControlError::new(
                AppControlErrorCode::AccessibilityPermissionMissing,
                "Accessibility permission changed before the app action.",
            ));
        }
        if request.cancellation.cancelled() {
            return Err(stale_action());
        }
        let context = focused_context()?;
        if context.pid as u64 != request.application.process_id
            || context.bundle_id != request.application.bundle_id
            || context.window.key() != request.window.window_id
        {
            return Err(AppControlError::new(
                AppControlErrorCode::CrossApplicationReference,
                "The active application changed before the action.",
            ));
        }
        if copy_bool(&context.window, "AXMinimized").unwrap_or(false) {
            return Err(AppControlError::new(
                AppControlErrorCode::HiddenWindow,
                "The application window is hidden.",
            ));
        }

        let targets = {
            let registry = self
                .registry
                .lock()
                .map_err(|_| driver_error("The Accessibility target registry is unavailable."))?;
            request
                .targets
                .iter()
                .map(|target| {
                    registry
                        .get(&target.element_key)
                        .filter(|entry| entry.session_id == request.session_id)
                        .map(|entry| entry.element.clone())
                        .ok_or_else(stale_action)
                })
                .collect::<AppControlResult<Vec<_>>>()?
        };

        if request.cancellation.cancelled() {
            return Err(stale_action());
        }
        match &request.action {
            DesktopSemanticAction::Focus { .. } => {
                set_boolean(target(&targets)?, "AXFocused", true, &request.cancellation)?
            }
            DesktopSemanticAction::Press { .. } => {
                perform_ax_action(target(&targets)?, "AXPress", &request.cancellation)?
            }
            DesktopSemanticAction::Select { value, .. }
            | DesktopSemanticAction::TypeText { text: value, .. } => {
                set_string(target(&targets)?, "AXValue", value, &request.cancellation)?
            }
            DesktopSemanticAction::InvokeMenu { command } => {
                super::macos_actions::perform_menu_shortcut(*command, &request.cancellation)?
            }
            DesktopSemanticAction::Scroll { amount, .. } => {
                super::macos_actions::perform_scroll(*amount, &request.cancellation)?
            }
            DesktopSemanticAction::DragDrop { .. } => {
                let (source, destination) = target_pair(&request.targets)?;
                super::macos_actions::perform_drag_drop(
                    source.geometry.ok_or_else(stale_action)?,
                    destination.geometry.ok_or_else(stale_action)?,
                    &request.cancellation,
                )?
            }
            DesktopSemanticAction::ChooseFile { .. } => {
                perform_file_choice(request, target(&targets)?)?
            }
            DesktopSemanticAction::AppleEvent { command } => super::macos_apple_event::perform(
                *command,
                request.application.process_id,
                &request.cancellation,
            )?,
        }

        let after_window_id = if request.expected_outcome == ExpectedOutcomeKind::WindowState {
            let after = focused_context()?;
            if after.pid as u64 != request.application.process_id
                || after.bundle_id != request.application.bundle_id
            {
                return Err(driver_error(
                    "The application changed while verifying its window.",
                ));
            }
            Some(after.window.key())
        } else {
            None
        };
        let postcondition =
            super::macos_postcondition::action_postcondition(request, after_window_id)?;
        Ok(DriverActionResult {
            receipt_token: opaque_id("axreceipt"),
            postcondition,
        })
    }
}

struct FocusedContext {
    application: AxElement,
    window: AxElement,
    pid: i32,
    bundle_id: String,
    display_name: String,
}

fn focused_context() -> AppControlResult<FocusedContext> {
    // SAFETY: the create function returns a +1 AX object or null.
    let system = OwnedCf::from_created(unsafe { AXUIElementCreateSystemWide() })
        .ok_or_else(|| driver_error("macOS did not provide the Accessibility root."))?;
    let system = AxElement::from_owned(system)?;
    let application = copy_element(&system, "AXFocusedApplication")?;
    let window = copy_element(&application, "AXFocusedWindow")?;
    let mut pid = 0_i32;
    // SAFETY: `pid` is valid writable memory and `application` is retained.
    if unsafe { AXUIElementGetPid(application.as_ptr(), &mut pid) } != AX_SUCCESS || pid <= 0 {
        return Err(driver_error(
            "The active application identity is unavailable.",
        ));
    }
    let (bundle_id, display_name) = super::macos_process::process_bundle(pid);
    Ok(FocusedContext {
        application,
        window,
        pid,
        bundle_id,
        display_name,
    })
}

struct TreeCollector {
    session_id: String,
    focused_key: Option<String>,
    elements: Vec<DriverElement>,
    registry: HashMap<String, RegisteredElement>,
    visited: HashSet<usize>,
}

impl TreeCollector {
    fn new(session_id: String, focused_key: Option<String>) -> Self {
        Self {
            session_id,
            focused_key,
            elements: Vec::new(),
            registry: HashMap::new(),
            visited: HashSet::new(),
        }
    }

    fn walk(
        &mut self,
        element: &AxElement,
        inherited_modal: bool,
        depth: usize,
    ) -> AppControlResult<()> {
        if depth > MAX_TREE_DEPTH || self.elements.len() >= MAX_NATIVE_ELEMENTS {
            return Ok(());
        }
        let address = element.as_ptr() as usize;
        if !self.visited.insert(address) {
            return Ok(());
        }
        let role = copy_string(element, "AXRole").unwrap_or_else(|| "AXUnknown".to_string());
        let subrole = copy_string(element, "AXSubrole").unwrap_or_default();
        let secure = role == "AXSecureTextField" || subrole == "AXSecureTextField";
        let visible = copy_bool(element, "AXVisible").unwrap_or(true);
        let enabled = copy_bool(element, "AXEnabled").unwrap_or(true);
        let in_modal = inherited_modal || copy_bool(element, "AXModal").unwrap_or(false);
        let settable_value = attribute_settable(element, "AXValue");
        let action_names = copy_action_names(element);
        let geometry = copy_geometry(element);
        let mut supported_actions = Vec::new();
        if visible && enabled {
            supported_actions.push(DesktopActionKind::Focus);
            if action_names.contains("AXPress") {
                supported_actions.push(DesktopActionKind::Press);
            }
            if settable_value {
                supported_actions.push(DesktopActionKind::Select);
                if !secure && text_role(&role) {
                    supported_actions.push(DesktopActionKind::TypeText);
                }
            }
            if role == "AXScrollArea" {
                supported_actions.push(DesktopActionKind::Scroll);
            }
            if geometry.is_some() && drag_role(&role) {
                supported_actions.push(DesktopActionKind::DragDrop);
            }
            if in_modal
                && settable_value
                && file_choice_role(&role)
                && action_names.contains("AXConfirm")
            {
                supported_actions.push(DesktopActionKind::ChooseFile);
            }
        }
        let title = copy_string(element, "AXTitle")
            .or_else(|| copy_string(element, "AXDescription"))
            .or_else(|| copy_string(element, "AXHelp"));
        let value_digest = if secure {
            None
        } else {
            copy_string(element, "AXValue")
                .map(|value| hex::encode(Sha256::digest(value.as_bytes())))
        };
        let key = element.key();
        self.registry.insert(
            key.clone(),
            RegisteredElement {
                session_id: self.session_id.clone(),
                element: element.clone(),
            },
        );
        self.elements.push(DriverElement {
            element_key: key,
            role,
            label: title,
            value_digest,
            secure,
            visible,
            enabled,
            in_modal,
            supported_actions,
            geometry,
        });
        for child in copy_elements(element, "AXChildren") {
            self.walk(&child, in_modal, depth + 1)?;
            if self.elements.len() >= MAX_NATIVE_ELEMENTS {
                break;
            }
        }
        Ok(())
    }
}

fn copy_value(element: &AxElement, attribute: &str) -> Option<OwnedCf> {
    let attribute = cf_string(attribute)?;
    let mut value: CFTypeRef = ptr::null();
    // SAFETY: pointers are retained and `value` is a valid out parameter.
    let status =
        unsafe { AXUIElementCopyAttributeValue(element.as_ptr(), attribute.as_ptr(), &mut value) };
    (status == AX_SUCCESS)
        .then(|| OwnedCf::from_created(value))
        .flatten()
}

fn copy_element(element: &AxElement, attribute: &str) -> AppControlResult<AxElement> {
    AxElement::from_owned(
        copy_value(element, attribute)
            .ok_or_else(|| driver_error("The focused Accessibility element is unavailable."))?,
    )
}

fn copy_string(element: &AxElement, attribute: &str) -> Option<String> {
    copy_value(element, attribute).and_then(|value| string_from_cf(value.as_ptr()))
}

fn copy_bool(element: &AxElement, attribute: &str) -> Option<bool> {
    let value = copy_value(element, attribute)?;
    // SAFETY: type ID checks precede the typed getter.
    (unsafe { CFGetTypeID(value.as_ptr()) } == unsafe { CFBooleanGetTypeID() })
        .then(|| unsafe { CFBooleanGetValue(value.as_ptr()) })
}

fn copy_geometry(element: &AxElement) -> Option<super::contracts::ElementGeometry> {
    let position = copy_value(element, "AXPosition")?;
    let size = copy_value(element, "AXSize")?;
    // SAFETY: both objects remain retained for the duration of the decoder call.
    unsafe { super::macos_geometry::decode(position.as_ptr(), size.as_ptr()) }
}

fn copy_elements(element: &AxElement, attribute: &str) -> Vec<AxElement> {
    let Some(value) = copy_value(element, attribute) else {
        return Vec::new();
    };
    // SAFETY: the retained object remains live throughout array traversal.
    if unsafe { CFGetTypeID(value.as_ptr()) } != unsafe { CFArrayGetTypeID() } {
        return Vec::new();
    }
    let count = unsafe { CFArrayGetCount(value.as_ptr()) }.clamp(0, MAX_NATIVE_ELEMENTS as c_long);
    (0..count)
        .filter_map(|index| {
            // SAFETY: index is within the reported array bounds; retain creates ownership.
            let item = unsafe { CFArrayGetValueAtIndex(value.as_ptr(), index) };
            (!item.is_null())
                .then(|| OwnedCf(unsafe { CFRetain(item) }))
                .and_then(|item| AxElement::from_owned(item).ok())
        })
        .collect()
}

fn copy_action_names(element: &AxElement) -> HashSet<String> {
    let mut value: CFArrayRef = ptr::null();
    // SAFETY: `value` is a valid out parameter and `element` is retained.
    if unsafe { AXUIElementCopyActionNames(element.as_ptr(), &mut value) } != AX_SUCCESS {
        return HashSet::new();
    }
    let Some(value) = OwnedCf::from_created(value) else {
        return HashSet::new();
    };
    let count = unsafe { CFArrayGetCount(value.as_ptr()) }.clamp(0, 128);
    (0..count)
        .filter_map(|index| {
            let item = unsafe { CFArrayGetValueAtIndex(value.as_ptr(), index) };
            string_from_cf(item)
        })
        .collect()
}

fn attribute_settable(element: &AxElement, attribute: &str) -> bool {
    let Some(attribute) = cf_string(attribute) else {
        return false;
    };
    let mut settable = false;
    // SAFETY: pointers are retained and `settable` is writable.
    unsafe {
        AXUIElementIsAttributeSettable(element.as_ptr(), attribute.as_ptr(), &mut settable)
            == AX_SUCCESS
            && settable
    }
}

fn cf_string(value: &str) -> Option<OwnedCf> {
    let mut bytes = value.as_bytes().to_vec();
    if bytes.contains(&0) {
        return None;
    }
    bytes.push(0);
    // SAFETY: bytes are NUL-terminated and live for the duration of the call.
    OwnedCf::from_created(unsafe {
        CFStringCreateWithCString(ptr::null(), bytes.as_ptr().cast(), UTF8_ENCODING)
    })
}

fn string_from_cf(value: CFTypeRef) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    let length = unsafe { CFStringGetLength(value) };
    let capacity =
        unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8_ENCODING) }.saturating_add(1);
    if capacity <= 0 || capacity > 1024 * 1024 {
        return None;
    }
    let mut buffer = vec![0_u8; capacity as usize];
    let copied =
        unsafe { CFStringGetCString(value, buffer.as_mut_ptr().cast(), capacity, UTF8_ENCODING) };
    copied.then(|| {
        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..end]).to_string()
    })
}

fn set_boolean(
    element: &AxElement,
    attribute: &str,
    value: bool,
    cancellation: &super::driver::DriverCancellationToken,
) -> AppControlResult<()> {
    if !value || cancellation.cancelled() {
        return Err(stale_action());
    }
    let attribute = cf_string(attribute).ok_or_else(|| driver_error("Invalid attribute."))?;
    // SAFETY: the semantic adapter only sets a fixed Boolean Accessibility attribute.
    let status = unsafe {
        AXUIElementSetAttributeValue(element.as_ptr(), attribute.as_ptr(), kCFBooleanTrue)
    };
    ax_status(status)
}

fn set_string(
    element: &AxElement,
    attribute: &str,
    value: &str,
    cancellation: &super::driver::DriverCancellationToken,
) -> AppControlResult<()> {
    let attribute = cf_string(attribute).ok_or_else(|| driver_error("Invalid attribute."))?;
    let value = cf_string(value).ok_or_else(|| driver_error("Invalid text value."))?;
    if cancellation.cancelled() {
        return Err(stale_action());
    }
    // SAFETY: both values are retained CF strings and the attribute is fixed by the adapter.
    ax_status(unsafe {
        AXUIElementSetAttributeValue(element.as_ptr(), attribute.as_ptr(), value.as_ptr())
    })
}

fn perform_ax_action(
    element: &AxElement,
    action: &str,
    cancellation: &super::driver::DriverCancellationToken,
) -> AppControlResult<()> {
    let action = cf_string(action).ok_or_else(|| driver_error("Invalid action."))?;
    if cancellation.cancelled() {
        return Err(stale_action());
    }
    // SAFETY: action is selected from the fixed semantic adapter table.
    ax_status(unsafe { AXUIElementPerformAction(element.as_ptr(), action.as_ptr()) })
}

fn target(elements: &[AxElement]) -> AppControlResult<&AxElement> {
    elements.first().ok_or_else(stale_action)
}

fn target_pair(
    elements: &[super::driver::ResolvedDriverTarget],
) -> AppControlResult<(
    &super::driver::ResolvedDriverTarget,
    &super::driver::ResolvedDriverTarget,
)> {
    match elements {
        [source, destination] => Ok((source, destination)),
        _ => Err(stale_action()),
    }
}

fn perform_file_choice(request: &DriverActionRequest, element: &AxElement) -> AppControlResult<()> {
    let resolved = request.targets.first().ok_or_else(stale_action)?;
    if !request.window.modal || !resolved.in_modal || !file_choice_role(&resolved.role) {
        return Err(AppControlError::new(
            AppControlErrorCode::AmbiguousTarget,
            "File selection requires a fresh native file dialog target.",
        ));
    }
    let path = request.selected_file.as_ref().ok_or_else(|| {
        AppControlError::new(
            AppControlErrorCode::FileScopeViolation,
            "The picker-issued file grant is unavailable.",
        )
    })?;
    let value = path.to_str().ok_or_else(|| {
        AppControlError::new(
            AppControlErrorCode::FileScopeViolation,
            "The selected file path cannot be represented safely.",
        )
    })?;
    if value.chars().count() > 4_096 || !copy_action_names(element).contains("AXConfirm") {
        return Err(AppControlError::new(
            AppControlErrorCode::AmbiguousTarget,
            "The native file dialog does not expose the qualified selection action.",
        ));
    }
    set_string(element, "AXValue", value, &request.cancellation)?;
    perform_ax_action(element, "AXConfirm", &request.cancellation)
}

fn ax_status(status: AXError) -> AppControlResult<()> {
    if status == AX_SUCCESS {
        Ok(())
    } else {
        Err(driver_error(format!(
            "macOS rejected the bounded Accessibility action ({status})."
        )))
    }
}

fn trusted() -> bool {
    // SAFETY: permission query has no parameters or side effects.
    unsafe { AXIsProcessTrusted() }
}

fn text_role(role: &str) -> bool {
    matches!(
        role,
        "AXTextField" | "AXTextArea" | "AXSearchField" | "AXComboBox"
    )
}

fn file_choice_role(role: &str) -> bool {
    matches!(role, "AXTextField" | "AXComboBox" | "AXSearchField")
}

fn drag_role(role: &str) -> bool {
    matches!(
        role,
        "AXCell" | "AXGroup" | "AXImage" | "AXOutline" | "AXRow" | "AXScrollArea"
    )
}

fn driver_error(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::DriverFailure, message)
}

fn stale_action() -> AppControlError {
    AppControlError::new(
        AppControlErrorCode::StaleReference,
        "The screen changed before the app action could commit.",
    )
}
