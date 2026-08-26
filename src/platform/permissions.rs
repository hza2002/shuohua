#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    Authorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePermissionState {
    Ready,
    ActionRequired(RuntimePermission),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePermission {
    Microphone,
    Accessibility,
}

#[cfg(target_os = "macos")]
pub fn accessibility_trusted() -> bool {
    crate::platform::macos::permissions::accessibility_trusted()
}

#[cfg(target_os = "macos")]
pub async fn preflight_runtime_permissions() -> RuntimePermissionState {
    crate::platform::macos::permissions::preflight_runtime_permissions().await
}

#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub async fn preflight_runtime_permissions() -> RuntimePermissionState {
    RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
}

#[cfg(target_os = "macos")]
pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    crate::platform::macos::permissions::microphone_authorization()
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    None
}
