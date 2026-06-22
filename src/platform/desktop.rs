use anyhow::Result;

use crate::post::AppContext;

pub(crate) use crate::platform::permissions::MicrophoneAuthorization;

#[cfg(target_os = "macos")]
pub(crate) fn frontmost_app() -> AppContext {
    crate::platform::macos::app_context::frontmost_app()
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn frontmost_app() -> AppContext {
    AppContext::default()
}

pub(crate) fn write_clipboard_string(text: &str) -> Result<()> {
    crate::platform::clipboard::write_string(text)
}

pub(crate) fn paste_text() -> Result<()> {
    crate::platform::autotype::paste()
}

pub(crate) fn accessibility_trusted() -> bool {
    crate::platform::permissions::accessibility_trusted()
}

pub(crate) fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    crate::platform::permissions::microphone_authorization()
}
