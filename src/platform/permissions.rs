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

#[cfg(target_os = "macos")]
pub fn request_accessibility_trust() -> bool {
    crate::platform::macos::permissions::request_accessibility_trust()
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn request_accessibility_trust() -> bool {
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
