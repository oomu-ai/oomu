use super::contracts::ElementGeometry;
use std::ffi::{c_long, c_void};

const AX_VALUE_CG_POINT: c_long = 1;
const AX_VALUE_CG_SIZE: c_long = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CgSize {
    width: f64,
    height: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXValueGetType(value: *const c_void) -> c_long;
    fn AXValueGetValue(value: *const c_void, value_type: c_long, output: *mut c_void) -> bool;
}

/// Decodes retained Accessibility geometry values into bounded screen-space data.
///
/// # Safety
/// `position` and `size` must be live AXValue objects for the duration of this call.
pub(super) unsafe fn decode(
    position: *const c_void,
    size: *const c_void,
) -> Option<ElementGeometry> {
    if unsafe { AXValueGetType(position) } != AX_VALUE_CG_POINT
        || unsafe { AXValueGetType(size) } != AX_VALUE_CG_SIZE
    {
        return None;
    }
    let mut point = CgPoint::default();
    let mut dimensions = CgSize::default();
    if !unsafe {
        AXValueGetValue(
            position,
            AX_VALUE_CG_POINT,
            (&mut point as *mut CgPoint).cast(),
        )
    } || !unsafe {
        AXValueGetValue(
            size,
            AX_VALUE_CG_SIZE,
            (&mut dimensions as *mut CgSize).cast(),
        )
    } {
        return None;
    }
    let values = [point.x, point.y, dimensions.width, dimensions.height];
    if values.iter().any(|value| !value.is_finite())
        || dimensions.width <= 0.0
        || dimensions.height <= 0.0
        || dimensions.width > 100_000.0
        || dimensions.height > 100_000.0
    {
        return None;
    }
    Some(ElementGeometry {
        x: point.x,
        y: point.y,
        width: dimensions.width,
        height: dimensions.height,
    })
}
