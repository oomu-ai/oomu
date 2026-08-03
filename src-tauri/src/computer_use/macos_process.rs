use std::{ffi::c_void, path::Path, ptr};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFIndex = std::ffi::c_long;
const UTF8_ENCODING: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: CFTypeRef);
    fn CFGetTypeID(value: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        value: CFStringRef,
        buffer: *mut std::ffi::c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> bool;
    fn CFURLCreateFromFileSystemRepresentation(
        allocator: *const c_void,
        bytes: *const u8,
        length: CFIndex,
        is_directory: bool,
    ) -> CFTypeRef;
    fn CFBundleCreate(allocator: *const c_void, url: CFTypeRef) -> CFTypeRef;
    fn CFBundleGetIdentifier(bundle: CFTypeRef) -> CFStringRef;
}

extern "C" {
    fn proc_pidpath(pid: std::ffi::c_int, buffer: *mut c_void, buffer_size: u32) -> i32;
}

struct OwnedCf(CFTypeRef);

impl OwnedCf {
    fn new(value: CFTypeRef) -> Option<Self> {
        (!value.is_null()).then_some(Self(value))
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        // SAFETY: the wrapper owns one create reference.
        unsafe { CFRelease(self.0) };
    }
}

pub(super) fn process_bundle(pid: i32) -> (String, String) {
    let mut buffer = vec![0_u8; 4096];
    // SAFETY: buffer is writable for its declared size.
    let length = unsafe {
        proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            buffer.len().try_into().unwrap_or(u32::MAX),
        )
    };
    if length <= 0 {
        return ("unknown.process".to_string(), "Application".to_string());
    }
    let executable = String::from_utf8_lossy(&buffer[..length as usize]).to_string();
    let app_path = executable
        .rfind(".app/")
        .map(|index| &executable[..index + 4]);
    let display_name = app_path
        .and_then(|path| Path::new(path).file_stem())
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Application".to_string());
    let bundle_id = app_path
        .and_then(bundle_identifier)
        .unwrap_or_else(|| "unknown.process".to_string());
    (bundle_id, display_name)
}

fn bundle_identifier(path: &str) -> Option<String> {
    // SAFETY: the URL and bundle stay retained during the borrowed identifier read.
    unsafe {
        let url = OwnedCf::new(CFURLCreateFromFileSystemRepresentation(
            ptr::null(),
            path.as_ptr(),
            path.len() as CFIndex,
            true,
        ))?;
        let bundle = OwnedCf::new(CFBundleCreate(ptr::null(), url.0))?;
        string_from_cf(CFBundleGetIdentifier(bundle.0))
    }
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
