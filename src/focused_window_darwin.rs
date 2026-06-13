use core::ffi::c_void;

use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGSize};
use objc2_foundation::{NSPoint, NSRect, NSSize};

type AXError = i32;
type AXUIElementRef = *const c_void;
type AXValueRef = *const c_void;

const AX_ERROR_SUCCESS: AXError = 0;
const AX_VALUE_CGPOINT: i32 = 1;
const AX_VALUE_CGSIZE: i32 = 2;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXValueGetValue(value: AXValueRef, the_type: i32, out: *mut c_void) -> bool;
}

pub fn focused_window_frame(screen_height: f64) -> Option<NSRect> {
    let system = unsafe { AXUIElementCreateSystemWide() };
    if system.is_null() {
        return None;
    }

    let window = copy_attr(system, "AXFocusedWindow");
    unsafe { CFRelease(system) };

    let window = window?;
    let position = copy_attr(window as AXUIElementRef, "AXPosition");
    let size = copy_attr(window as AXUIElementRef, "AXSize");
    unsafe { CFRelease(window) };

    let (position, size) = match (position, size) {
        (Some(position), Some(size)) => (position, size),
        (position, size) => {
            if let Some(position) = position {
                unsafe { CFRelease(position) };
            }
            if let Some(size) = size {
                unsafe { CFRelease(size) };
            }
            return None;
        }
    };

    let mut point = CGPoint::new(0.0, 0.0);
    let mut cg_size = CGSize::new(0.0, 0.0);
    let got_point = unsafe {
        AXValueGetValue(
            position as AXValueRef,
            AX_VALUE_CGPOINT,
            (&mut point as *mut CGPoint).cast(),
        )
    };
    let got_size = unsafe {
        AXValueGetValue(
            size as AXValueRef,
            AX_VALUE_CGSIZE,
            (&mut cg_size as *mut CGSize).cast(),
        )
    };
    unsafe {
        CFRelease(position);
        CFRelease(size);
    }

    if !got_point || !got_size || cg_size.width <= 0.0 || cg_size.height <= 0.0 {
        return None;
    }

    let appkit_y = screen_height - point.y - cg_size.height;
    Some(NSRect::new(
        NSPoint::new(point.x, appkit_y),
        NSSize::new(cg_size.width, cg_size.height),
    ))
}

fn copy_attr(element: AXUIElementRef, attr: &str) -> Option<CFTypeRef> {
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err =
        unsafe { AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value) };
    if err == AX_ERROR_SUCCESS && !value.is_null() {
        Some(value)
    } else {
        None
    }
}
