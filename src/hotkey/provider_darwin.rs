//! CGEventTap → pipe bridge + foreground-app suppress.
//!
//! docs/DESIGN.md §2.4 + §5 invariant 8: the tap runs in `Default` mode so the
//! callback can decide to *drop* an event (return `CallbackResult::Drop` → the
//! safe wrapper returns NULL → the OS does not forward the event to other
//! consumers). We always also write the event to the pipe so the tokio-side
//! `Tracker` keeps seeing every keypress, regardless of suppression.
//!
//! Suppress decisions live in [`Suppressor`] — a pure state machine that
//! tracks the "physical keys we've eaten so far" so the matching KeyUp is
//! suppressed even when the trigger is re-bound mid-hold.

use anyhow::{anyhow, Result};
use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use os_pipe::PipeWriter;
use std::io::Write;
use std::sync::{Arc, Mutex};

use super::{RawKey, Suppressor};

/// Install a CGEventTap on the current thread's CFRunLoop and block until the
/// runloop stops. Every observed KeyDown/KeyUp is encoded to the pipe.
/// Suppression is delegated to `suppressor` (shared with daemon main loop so
/// `[hotkey].trigger` reloads can update the trigger code without restarting
/// the tap).
pub fn run(writer: PipeWriter, suppressor: Arc<Mutex<Suppressor>>) -> Result<()> {
    let pipe = Mutex::new(writer);

    CGEventTap::with_enabled(
        // Session level matches our M1 design. HID-level taps need stronger
        // entitlements and aren't needed here.
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        // Default = active filter: callback's return value decides whether
        // the event continues to other consumers. ListenOnly would ignore it.
        CGEventTapOptions::Default,
        vec![CGEventType::KeyDown, CGEventType::KeyUp],
        move |_proxy, etype, event| {
            let down = matches!(etype, CGEventType::KeyDown);
            let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;

            // Forward unconditionally — the Tracker on the tokio side must see
            // every event, including ones we eat for the foreground app.
            let buf = RawKey::encode(down, code);
            if let Ok(mut w) = pipe.lock() {
                let _ = w.write_all(&buf);
            }

            let drop_event = match suppressor.lock() {
                Ok(mut s) => s.on_raw(RawKey { down, code }),
                // Poisoned mutex means a panic happened on a Suppressor user;
                // safer to let events through than to silently eat them.
                Err(_) => false,
            };

            if drop_event {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
        || {
            CFRunLoop::run_current();
        },
    )
    .map_err(|_| {
        anyhow!(
            "CGEventTapCreate failed. Default-mode taps require Accessibility \
             permission — grant it to the terminal running `shuo` in System \
             Settings → Privacy & Security → Accessibility."
        )
    })?;

    Ok(())
}
