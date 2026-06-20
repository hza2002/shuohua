use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::ns_string;

use crate::platform::permissions::MicrophoneAuthorization;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
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
