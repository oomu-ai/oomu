//! Closed macOS input emitters for already-qualified semantic actions.

use super::{
    contracts::{ElementGeometry, QualifiedMenuCommand},
    driver::DriverCancellationToken,
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use std::{ffi::c_void, ptr};

type CGEventRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CgPoint {
    x: f64,
    y: f64,
}

const COMMAND_FLAG: u64 = 1 << 20;
const SHIFT_FLAG: u64 = 1 << 17;
const HID_EVENT_TAP: u32 = 0;
const SCROLL_UNIT_LINE: u32 = 1;
const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const MOUSE_MOVED: u32 = 5;
const LEFT_MOUSE_DRAGGED: u32 = 6;
const LEFT_MOUSE_BUTTON: u32 = 0;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CGEventCreateScrollWheelEvent(
        source: *const c_void,
        units: u32,
        wheel_count: u32,
        ...
    ) -> CGEventRef;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: u32,
        mouse_cursor_position: CgPoint,
        mouse_button: u32,
    ) -> CGEventRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

struct OwnedEvent(CGEventRef);

impl OwnedEvent {
    fn from_created(value: CGEventRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        // SAFETY: each wrapper owns one CoreFoundation create reference.
        unsafe { CFRelease(self.0) };
    }
}

pub(super) fn perform_menu_shortcut(
    command: QualifiedMenuCommand,
    cancellation: &DriverCancellationToken,
) -> AppControlResult<()> {
    let (key, flags) = match command {
        QualifiedMenuCommand::Save => (1, COMMAND_FLAG),
        QualifiedMenuCommand::SaveAs => (1, COMMAND_FLAG | SHIFT_FLAG),
        QualifiedMenuCommand::NewWindow => (45, COMMAND_FLAG),
        QualifiedMenuCommand::CloseWindow => (13, COMMAND_FLAG),
        QualifiedMenuCommand::Export => {
            return Err(driver_error(
                "Export needs a reviewed app-specific adapter.",
            ))
        }
    };
    ensure_current(cancellation)?;
    // SAFETY: only the fixed virtual keys above reach this native seam.
    unsafe {
        let down = OwnedEvent::from_created(CGEventCreateKeyboardEvent(ptr::null(), key, true))
            .ok_or_else(|| driver_error("macOS could not create the menu command."))?;
        let up = OwnedEvent::from_created(CGEventCreateKeyboardEvent(ptr::null(), key, false))
            .ok_or_else(|| driver_error("macOS could not create the menu command."))?;
        CGEventSetFlags(down.0, flags);
        CGEventSetFlags(up.0, flags);
        ensure_current(cancellation)?;
        CGEventPost(HID_EVENT_TAP, down.0);
        CGEventPost(HID_EVENT_TAP, up.0);
    }
    Ok(())
}

pub(super) fn perform_scroll(
    amount: i32,
    cancellation: &DriverCancellationToken,
) -> AppControlResult<()> {
    ensure_current(cancellation)?;
    // SAFETY: this emits one bounded axis; the manager constrains `amount`.
    unsafe {
        let event = OwnedEvent::from_created(CGEventCreateScrollWheelEvent(
            ptr::null(),
            SCROLL_UNIT_LINE,
            1,
            amount,
        ))
        .ok_or_else(|| driver_error("macOS could not create the scroll action."))?;
        ensure_current(cancellation)?;
        CGEventPost(HID_EVENT_TAP, event.0);
    }
    Ok(())
}

pub(super) fn perform_drag_drop(
    source: ElementGeometry,
    destination: ElementGeometry,
    cancellation: &DriverCancellationToken,
) -> AppControlResult<()> {
    let source = geometry_center(source)?;
    let destination = geometry_center(destination)?;
    post_mouse(MOUSE_MOVED, source, cancellation)?;
    post_mouse(LEFT_MOUSE_DOWN, source, cancellation)?;
    if let Err(error) = post_mouse(LEFT_MOUSE_DRAGGED, destination, cancellation) {
        post_mouse_unchecked(LEFT_MOUSE_UP, source);
        return Err(error);
    }
    if let Err(error) = post_mouse(LEFT_MOUSE_UP, destination, cancellation) {
        post_mouse_unchecked(LEFT_MOUSE_UP, destination);
        return Err(error);
    }
    Ok(())
}

fn post_mouse(
    event_type: u32,
    point: CgPoint,
    cancellation: &DriverCancellationToken,
) -> AppControlResult<()> {
    ensure_current(cancellation)?;
    let event = create_mouse_event(event_type, point)?;
    ensure_current(cancellation)?;
    // SAFETY: the event is retained and the tap is the fixed local HID tap.
    unsafe { CGEventPost(HID_EVENT_TAP, event.0) };
    Ok(())
}

fn post_mouse_unchecked(event_type: u32, point: CgPoint) {
    if let Ok(event) = create_mouse_event(event_type, point) {
        // SAFETY: cleanup emits only a fixed mouse-up event after a bounded drag.
        unsafe { CGEventPost(HID_EVENT_TAP, event.0) };
    }
}

fn create_mouse_event(event_type: u32, point: CgPoint) -> AppControlResult<OwnedEvent> {
    // SAFETY: event type, button, and point come only from this closed adapter.
    OwnedEvent::from_created(unsafe {
        CGEventCreateMouseEvent(ptr::null(), event_type, point, LEFT_MOUSE_BUTTON)
    })
    .ok_or_else(|| driver_error("macOS could not create the drag action."))
}

fn geometry_center(geometry: ElementGeometry) -> AppControlResult<CgPoint> {
    let values = [geometry.x, geometry.y, geometry.width, geometry.height];
    if values.iter().any(|value| !value.is_finite())
        || geometry.width <= 0.0
        || geometry.height <= 0.0
        || geometry.width > 100_000.0
        || geometry.height > 100_000.0
    {
        return Err(driver_error("The observed drag bounds are invalid."));
    }
    let point = CgPoint {
        x: geometry.x + geometry.width / 2.0,
        y: geometry.y + geometry.height / 2.0,
    };
    if !(-100_000.0..=100_000.0).contains(&point.x) || !(-100_000.0..=100_000.0).contains(&point.y)
    {
        return Err(driver_error(
            "The observed drag target is outside the display bounds.",
        ));
    }
    Ok(point)
}

fn ensure_current(cancellation: &DriverCancellationToken) -> AppControlResult<()> {
    if cancellation.cancelled() {
        Err(AppControlError::new(
            AppControlErrorCode::StaleReference,
            "The screen changed before the app action could commit.",
        ))
    } else {
        Ok(())
    }
}

fn driver_error(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::DriverFailure, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_points_derive_only_from_observed_geometry() {
        assert_eq!(
            geometry_center(ElementGeometry {
                x: 10.0,
                y: 20.0,
                width: 40.0,
                height: 20.0,
            })
            .unwrap(),
            CgPoint { x: 30.0, y: 30.0 }
        );
        assert!(geometry_center(ElementGeometry {
            x: f64::NAN,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        })
        .is_err());
    }
}
