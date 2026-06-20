#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    crate::platform::macos::permissions::accessibility_trusted()
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    crate::platform::macos::permissions::microphone_authorization()
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    None
}
