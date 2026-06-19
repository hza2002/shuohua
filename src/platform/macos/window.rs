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

pub fn focused_window_frame_for_screens(screens: &[NSRect]) -> Option<(NSRect, NSRect)> {
    let (point, size) = focused_window_cg_frame()?;
    let cg_frame = NSRect::new(
        NSPoint::new(point.x, point.y),
        NSSize::new(size.width, size.height),
    );
    let screen = screen_containing_window(cg_frame, screens)?;
    Some((ax_frame_to_appkit(point, size, screen), screen))
}

fn focused_window_cg_frame() -> Option<(CGPoint, CGSize)> {
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

    Some((point, cg_size))
}

pub fn screen_containing_window(window: NSRect, screens: &[NSRect]) -> Option<NSRect> {
    screens.iter().copied().max_by(|a, b| {
        overlap_area(window, *a)
            .partial_cmp(&overlap_area(window, *b))
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

pub fn ax_frame_to_appkit(point: CGPoint, size: CGSize, screen: NSRect) -> NSRect {
    let appkit_y = screen.origin.y + screen.size.height - point.y - size.height;
    NSRect::new(
        NSPoint::new(point.x, appkit_y),
        NSSize::new(size.width, size.height),
    )
}

fn overlap_area(a: NSRect, b: NSRect) -> f64 {
    let left = a.origin.x.max(b.origin.x);
    let right = (a.origin.x + a.size.width).min(b.origin.x + b.size.width);
    let bottom = a.origin.y.max(b.origin.y);
    let top = (a.origin.y + a.size.height).min(b.origin.y + b.size.height);
    let w = (right - left).max(0.0);
    let h = (top - bottom).max(0.0);
    w * h
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn chooses_screen_with_largest_overlap_for_focused_window() {
        let screens = [
            rect(0.0, 0.0, 1440.0, 900.0),
            rect(1440.0, 0.0, 1440.0, 900.0),
        ];
        let window = rect(1500.0, 100.0, 800.0, 600.0);

        let chosen = screen_containing_window(window, &screens).unwrap();

        assert_eq!(chosen.origin.x, 1440.0);
    }

    #[test]
    fn converts_ax_y_using_target_screen_frame() {
        let screen = rect(0.0, 900.0, 1440.0, 900.0);
        let frame = ax_frame_to_appkit(
            CGPoint::new(120.0, 100.0),
            CGSize::new(800.0, 600.0),
            screen,
        );

        assert_eq!(frame.origin.x, 120.0);
        assert_eq!(frame.origin.y, 1100.0);
    }
}
