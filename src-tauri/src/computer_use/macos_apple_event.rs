//! Fixed Apple Event adapter for the already-observed application process.
//!
//! Event classes and identifiers are compile-time constants. This module has
//! no script, shell, descriptor-parameter, or caller-supplied event-code seam.

use super::{
    contracts::QualifiedAppleEvent,
    driver::DriverCancellationToken,
    error::{AppControlError, AppControlErrorCode, AppControlResult},
};
use std::{ffi::c_void, mem, ptr};

type OSStatus = i32;
type AEEventClass = u32;
type AEEventId = u32;
type DescType = u32;
type AESendMode = i32;

const NO_ERR: OSStatus = 0;
const TYPE_KERNEL_PROCESS_ID: DescType = u32::from_be_bytes(*b"kpid");
const CORE_EVENT_CLASS: AEEventClass = u32::from_be_bytes(*b"aevt");
const ACTIVATE_EVENT: AEEventId = u32::from_be_bytes(*b"actv");
const AUTO_GENERATE_RETURN_ID: i16 = -1;
const ANY_TRANSACTION_ID: i32 = 0;
const NO_REPLY: AESendMode = 1;
const DEFAULT_TIMEOUT_TICKS: i32 = 60;

#[repr(C)]
struct AEDesc {
    descriptor_type: DescType,
    data_handle: *mut c_void,
}

impl Default for AEDesc {
    fn default() -> Self {
        Self {
            descriptor_type: 0,
            data_handle: ptr::null_mut(),
        }
    }
}

struct OwnedDesc(AEDesc);

impl OwnedDesc {
    fn empty() -> Self {
        Self(AEDesc::default())
    }
}

impl Drop for OwnedDesc {
    fn drop(&mut self) {
        if self.0.descriptor_type != 0 || !self.0.data_handle.is_null() {
            // SAFETY: this wrapper owns one descriptor initialized by AECreate*.
            unsafe { AEDisposeDesc(&mut self.0) };
        }
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AECreateDesc(
        descriptor_type: DescType,
        data: *const c_void,
        size: isize,
        result: *mut AEDesc,
    ) -> OSStatus;
    fn AECreateAppleEvent(
        event_class: AEEventClass,
        event_id: AEEventId,
        target: *const AEDesc,
        return_id: i16,
        transaction_id: i32,
        result: *mut AEDesc,
    ) -> OSStatus;
    fn AESendMessage(
        event: *const AEDesc,
        reply: *mut AEDesc,
        send_mode: AESendMode,
        timeout_ticks: i32,
    ) -> OSStatus;
    fn AEDisposeDesc(descriptor: *mut AEDesc) -> OSStatus;
}

pub(super) fn perform(
    command: QualifiedAppleEvent,
    process_id: u64,
    cancellation: &DriverCancellationToken,
) -> AppControlResult<()> {
    let process_id = i32::try_from(process_id)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| driver_error("The observed application process is invalid."))?;
    let (event_class, event_id) = fixed_event(command);
    ensure_current(cancellation)?;

    let mut target = OwnedDesc::empty();
    status(unsafe {
        AECreateDesc(
            TYPE_KERNEL_PROCESS_ID,
            (&process_id as *const i32).cast(),
            mem::size_of::<i32>() as isize,
            &mut target.0,
        )
    })?;
    let mut event = OwnedDesc::empty();
    // SAFETY: codes are fixed by `fixed_event`; target is an owned PID descriptor.
    status(unsafe {
        AECreateAppleEvent(
            event_class,
            event_id,
            &target.0,
            AUTO_GENERATE_RETURN_ID,
            ANY_TRANSACTION_ID,
            &mut event.0,
        )
    })?;
    ensure_current(cancellation)?;
    // SAFETY: no-reply send uses an owned, fixed event and no result descriptor.
    status(unsafe { AESendMessage(&event.0, ptr::null_mut(), NO_REPLY, DEFAULT_TIMEOUT_TICKS) })
}

fn fixed_event(command: QualifiedAppleEvent) -> (AEEventClass, AEEventId) {
    match command {
        QualifiedAppleEvent::ActivateApplication => (CORE_EVENT_CLASS, ACTIVATE_EVENT),
    }
}

fn ensure_current(cancellation: &DriverCancellationToken) -> AppControlResult<()> {
    if cancellation.cancelled() {
        Err(AppControlError::new(
            AppControlErrorCode::StaleReference,
            "The screen changed before the app command could commit.",
        ))
    } else {
        Ok(())
    }
}

fn status(status: OSStatus) -> AppControlResult<()> {
    if status == NO_ERR {
        Ok(())
    } else {
        Err(driver_error(format!(
            "macOS rejected the bounded application command ({status})."
        )))
    }
}

fn driver_error(message: impl Into<String>) -> AppControlError {
    AppControlError::new(AppControlErrorCode::DriverFailure, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_event_codes_are_fixed_by_the_typed_command() {
        assert_eq!(
            fixed_event(QualifiedAppleEvent::ActivateApplication),
            (u32::from_be_bytes(*b"aevt"), u32::from_be_bytes(*b"actv"))
        );
    }
}
