use anyhow::{Context, Result};
use block2::RcBlock;
use core_foundation::base::{CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};
use objc2::msg_send;
use objc2::runtime::{AnyClass, Bool};
use objc2_avf_audio::AVAudioApplication;
use objc2_foundation::ns_string;

use crate::platform::permissions::{
    MicrophoneAuthorization, RuntimePermission, RuntimePermissionState,
};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub fn request_accessibility_trust() -> bool {
    let prompt_key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let prompt_value = CFBoolean::true_value();
    let options = CFDictionary::from_CFType_pairs(&[(prompt_key.as_CFType(), prompt_value)]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) }
}

pub async fn preflight_runtime_permissions() -> RuntimePermissionState {
    preflight_runtime_permissions_with(
        preflight_microphone_permission,
        preflight_accessibility_permission,
    )
    .await
}

async fn preflight_runtime_permissions_with<M, MFut, A, AFut>(
    microphone: M,
    accessibility: A,
) -> RuntimePermissionState
where
    M: FnOnce() -> MFut,
    MFut: std::future::Future<Output = RuntimePermissionState>,
    A: FnOnce() -> AFut,
    AFut: std::future::Future<Output = RuntimePermissionState>,
{
    let microphone = microphone().await;
    if microphone != RuntimePermissionState::Ready {
        return microphone;
    }
    accessibility().await
}

async fn preflight_microphone_permission() -> RuntimePermissionState {
    match microphone_authorization() {
        Some(MicrophoneAuthorization::Authorized) => RuntimePermissionState::Ready,
        Some(MicrophoneAuthorization::NotDetermined) => match request_microphone_access().await {
            Ok(true) => {
                tracing::info!("Microphone permission was granted");
                microphone_request_completed_state()
            }
            Ok(false) => {
                open_permission_settings("Microphone", "Privacy_Microphone");
                RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
            }
            Err(error) => {
                tracing::warn!(?error, "Microphone permission request did not complete");
                RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
            }
        },
        Some(MicrophoneAuthorization::Denied) => {
            open_permission_settings("Microphone", "Privacy_Microphone");
            record_permission_action(RuntimePermission::Microphone);
            RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
        }
        Some(MicrophoneAuthorization::Restricted) => {
            tracing::warn!("Microphone permission is restricted by system policy");
            open_permission_settings("Microphone", "Privacy_Microphone");
            record_permission_action(RuntimePermission::Microphone);
            RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
        }
        None => {
            tracing::warn!("unable to determine Microphone permission");
            open_permission_settings("Microphone", "Privacy_Microphone");
            record_permission_action(RuntimePermission::Microphone);
            RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
        }
    }
}

fn microphone_request_completed_state() -> RuntimePermissionState {
    RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
}

async fn preflight_accessibility_permission() -> RuntimePermissionState {
    preflight_accessibility_permission_with(
        accessibility_trusted,
        || {
            let _ = request_accessibility_trust();
        },
        || record_permission_action(RuntimePermission::Accessibility),
    )
}

fn preflight_accessibility_permission_with(
    trusted: impl FnOnce() -> bool,
    prompt: impl FnOnce(),
    record_action: impl FnOnce(),
) -> RuntimePermissionState {
    if trusted() {
        return RuntimePermissionState::Ready;
    }
    prompt();
    record_action();
    RuntimePermissionState::ActionRequired(RuntimePermission::Accessibility)
}

fn record_permission_action(permission: RuntimePermission) {
    if let Err(error) = crate::daemon::permission_outcome::write(permission) {
        tracing::warn!(
            ?permission,
            ?error,
            "failed to record required runtime permission"
        );
    }
}

async fn request_microphone_access() -> Result<bool> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let handler = RcBlock::new(move |granted: Bool| {
        if let Some(tx) = tx
            .lock()
            .expect("microphone permission callback lock")
            .take()
        {
            let _ = tx.send(granted.as_bool());
        }
    });
    unsafe {
        AVAudioApplication::requestRecordPermissionWithCompletionHandler(&handler);
    }
    record_permission_action(RuntimePermission::Microphone);
    rx.await
        .context("Microphone permission callback was dropped")
}

fn open_permission_settings(permission: &str, pane: &str) {
    match open_privacy_settings(pane) {
        Ok(()) => tracing::warn!(
            permission,
            "permission is required; enable shuo in System Settings, then start it again"
        ),
        Err(error) => tracing::warn!(
            permission,
            ?error,
            "permission is required, but System Settings could not be opened"
        ),
    }
}

fn open_privacy_settings(pane: &str) -> Result<()> {
    let url = privacy_settings_url(pane);
    let status = std::process::Command::new("/usr/bin/open")
        .arg(&url)
        .status()
        .with_context(|| format!("open System Settings pane {pane}"))?;
    anyhow::ensure!(
        status.success(),
        "open System Settings pane {pane} failed: {status}"
    );
    Ok(())
}

fn privacy_settings_url(pane: &str) -> String {
    format!("x-apple.systempreferences:com.apple.preference.security?{pane}")
}

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {}

pub fn microphone_authorization() -> Option<MicrophoneAuthorization> {
    let class = AnyClass::get(c"AVCaptureDevice")?;
    let status: isize =
        unsafe { msg_send![class, authorizationStatusForMediaType: ns_string!("soun")] };
    match status {
        0 => Some(MicrophoneAuthorization::NotDetermined),
        1 => Some(MicrophoneAuthorization::Restricted),
        2 => Some(MicrophoneAuthorization::Denied),
        3 => Some(MicrophoneAuthorization::Authorized),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_permissions_check_microphone_before_accessibility() {
        let calls = RefCell::new(Vec::new());

        let state = preflight_runtime_permissions_with(
            || async {
                calls.borrow_mut().push("microphone");
                RuntimePermissionState::Ready
            },
            || async {
                calls.borrow_mut().push("accessibility");
                RuntimePermissionState::Ready
            },
        )
        .await;

        assert_eq!(state, RuntimePermissionState::Ready);
        assert_eq!(&*calls.borrow(), &["microphone", "accessibility"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_permissions_stop_after_microphone_action_required() {
        let calls = RefCell::new(Vec::new());

        let state = preflight_runtime_permissions_with(
            || async {
                calls.borrow_mut().push("microphone");
                RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
            },
            || async {
                calls.borrow_mut().push("accessibility");
                RuntimePermissionState::Ready
            },
        )
        .await;

        assert_eq!(
            state,
            RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
        );
        assert_eq!(&*calls.borrow(), &["microphone"]);
    }

    #[test]
    fn privacy_settings_urls_target_exact_permission_panes() {
        assert_eq!(
            privacy_settings_url("Privacy_Microphone"),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        );
    }

    #[test]
    fn granted_microphone_prompt_still_ends_this_start_attempt() {
        assert_eq!(
            microphone_request_completed_state(),
            RuntimePermissionState::ActionRequired(RuntimePermission::Microphone)
        );
    }

    #[test]
    fn accessibility_uses_only_the_native_prompt_when_missing() {
        let calls = RefCell::new(Vec::new());

        let state = preflight_accessibility_permission_with(
            || {
                calls.borrow_mut().push("check");
                false
            },
            || calls.borrow_mut().push("prompt"),
            || calls.borrow_mut().push("record"),
        );

        assert_eq!(
            state,
            RuntimePermissionState::ActionRequired(RuntimePermission::Accessibility)
        );
        assert_eq!(&*calls.borrow(), &["check", "prompt", "record"]);
    }

    #[test]
    fn accessibility_skips_prompt_when_already_trusted() {
        let calls = RefCell::new(Vec::new());

        let state = preflight_accessibility_permission_with(
            || {
                calls.borrow_mut().push("check");
                true
            },
            || calls.borrow_mut().push("prompt"),
            || calls.borrow_mut().push("record"),
        );

        assert_eq!(state, RuntimePermissionState::Ready);
        assert_eq!(&*calls.borrow(), &["check"]);
    }
}
