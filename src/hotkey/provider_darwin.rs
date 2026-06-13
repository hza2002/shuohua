//! CGEventTap → pipe bridge.
//!
//! docs/DESIGN.md §5 invariant 2: C → Rust events go over a pipe, not a direct
//! callback to higher layers. The CGEventTap callback runs on the CFRunLoop
//! thread; we keep it allocation-light by only locking a Mutex<PipeWriter>
//! and doing one pipe write per keydown/keyup. At M1's human-rate keypress
//! frequency, Mutex contention is non-existent.

use anyhow::{anyhow, Result};
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    EventField,
};
use os_pipe::PipeWriter;
use std::io::Write;
use std::sync::Mutex;

use super::RawKey;

/// Block forever running a CGEventTap on the current thread's CFRunLoop.
/// Writes a 4-byte RawKey frame to `writer` for every observed keydown/keyup.
pub fn run(writer: PipeWriter) -> Result<()> {
    let writer = Mutex::new(writer);

    // Session level is the conventional choice for listening to keyboard
    // events; HID-level taps require the process to be a trusted Accessibility
    // client even in ListenOnly mode, which we don't need at M1.
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![CGEventType::KeyDown, CGEventType::KeyUp],
        move |_proxy, event_type, event| {
            let raw_code =
                event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            let down = matches!(event_type, CGEventType::KeyDown);
            let buf = RawKey::encode(down, raw_code);
            if let Ok(mut w) = writer.lock() {
                let _ = w.write_all(&buf);
            }
            None
        },
    )
    .map_err(|_| {
        anyhow!(
            "CGEventTap::new failed. Grant Accessibility (or Input Monitoring) to \
             the terminal running `shuo` in System Settings → Privacy & Security."
        )
    })?;

    unsafe {
        let source = tap
            .mach_port
            .create_runloop_source(0)
            .map_err(|_| anyhow!("create_runloop_source failed"))?;
        CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes);
        tap.enable();
        CFRunLoop::run_current();
    }

    Ok(())
}
