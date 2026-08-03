//! Listen-only physical input signal used to revoke in-flight app actions.

use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

type CGEventRef = *const c_void;
type CFMachPortRef = *const c_void;
type CFRunLoopSourceRef = *const c_void;
type CFRunLoopRef = *const c_void;
type CFStringRef = *const c_void;

const HID_EVENT_TAP: u32 = 0;
const HEAD_INSERT: u32 = 0;
const LISTEN_ONLY: u32 = 1;
const EVENT_SOURCE_UNIX_PROCESS_ID: u32 = 41;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const LEFT_MOUSE_DOWN: u32 = 1;
const LEFT_MOUSE_UP: u32 = 2;
const RIGHT_MOUSE_DOWN: u32 = 3;
const RIGHT_MOUSE_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_MOUSE_DRAGGED: u32 = 6;
const RIGHT_MOUSE_DRAGGED: u32 = 7;
const SCROLL_WHEEL: u32 = 22;
const OTHER_MOUSE_DOWN: u32 = 25;
const OTHER_MOUSE_UP: u32 = 26;
const OTHER_MOUSE_DRAGGED: u32 = 27;

type TapCallback = unsafe extern "C" fn(*const c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: TapCallback,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFRelease(value: *const c_void);
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRun();
}

#[derive(Clone)]
pub(super) struct PhysicalInputMonitor {
    pub epoch: Arc<AtomicU64>,
    pub ready: Arc<AtomicBool>,
}

pub(super) fn install_physical_input_monitor() -> PhysicalInputMonitor {
    let epoch = Arc::new(AtomicU64::new(1));
    let ready = Arc::new(AtomicBool::new(false));
    let thread_epoch = Arc::clone(&epoch);
    let thread_ready = Arc::clone(&ready);
    thread::Builder::new()
        .name("oomu-physical-input-monitor".to_string())
        .spawn(move || monitor_forever(thread_epoch, thread_ready))
        .ok();
    PhysicalInputMonitor { epoch, ready }
}

fn monitor_forever(epoch: Arc<AtomicU64>, ready: Arc<AtomicBool>) {
    loop {
        ready.store(false, Ordering::SeqCst);
        // Keep one strong reference alive for the callback's user pointer.
        let callback_epoch = Arc::into_raw(Arc::clone(&epoch));
        // SAFETY: the callback pointer remains valid and `callback_epoch` is held
        // until the run loop exits or tap construction fails.
        let tap = unsafe {
            CGEventTapCreate(
                HID_EVENT_TAP,
                HEAD_INSERT,
                LISTEN_ONLY,
                physical_event_mask(),
                input_callback,
                callback_epoch.cast_mut().cast(),
            )
        };
        if tap.is_null() {
            // SAFETY: tap construction did not retain the supplied user pointer.
            unsafe { drop(Arc::from_raw(callback_epoch)) };
            thread::sleep(Duration::from_secs(2));
            continue;
        }
        // SAFETY: tap and source are retained CoreFoundation objects.
        unsafe {
            let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
            if source.is_null() {
                CFRelease(tap);
                drop(Arc::from_raw(callback_epoch));
                thread::sleep(Duration::from_secs(2));
                continue;
            }
            CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
            CGEventTapEnable(tap, true);
            ready.store(true, Ordering::SeqCst);
            CFRunLoopRun();
            ready.store(false, Ordering::SeqCst);
            CFRelease(source);
            CFRelease(tap);
            drop(Arc::from_raw(callback_epoch));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

unsafe extern "C" fn input_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if event.is_null() || user_info.is_null() || !physical_event_type(event_type) {
        return event;
    }
    // Events synthesized by this process must not look like human takeover.
    let source_pid = unsafe { CGEventGetIntegerValueField(event, EVENT_SOURCE_UNIX_PROCESS_ID) };
    if source_pid != std::process::id() as i64 {
        let epoch = unsafe { &*(user_info.cast::<AtomicU64>()) };
        epoch.fetch_add(1, Ordering::SeqCst);
    }
    event
}

fn physical_event_mask() -> u64 {
    [
        KEY_DOWN,
        KEY_UP,
        FLAGS_CHANGED,
        LEFT_MOUSE_DOWN,
        LEFT_MOUSE_UP,
        RIGHT_MOUSE_DOWN,
        RIGHT_MOUSE_UP,
        MOUSE_MOVED,
        LEFT_MOUSE_DRAGGED,
        RIGHT_MOUSE_DRAGGED,
        SCROLL_WHEEL,
        OTHER_MOUSE_DOWN,
        OTHER_MOUSE_UP,
        OTHER_MOUSE_DRAGGED,
    ]
    .into_iter()
    .fold(0_u64, |mask, event_type| mask | (1_u64 << event_type))
}

fn physical_event_type(event_type: u32) -> bool {
    event_type < 64 && (physical_event_mask() & (1_u64 << event_type)) != 0
}
