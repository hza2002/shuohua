//! macOS 系统剪贴板写入。
//!
//! NSPasteboard.generalPasteboard 单例；先 clearContents() 清空 owner，
//! 再 setString:forType:NSPasteboardTypeString 写文本。
//!
//! 私有 API 风险：DESIGN §5 不变量没列剪贴板，因为 NSPasteboard 是公开 API。
//! 不上 App Store，objc2 互操作里偶尔出现的私有 selector 我们也敢用。

use anyhow::{anyhow, Result};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

pub fn write_string(text: &str) -> Result<()> {
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let ns_text = NSString::from_str(text);
        let ok = pb.setString_forType(&ns_text, NSPasteboardTypeString);
        if !ok {
            return Err(anyhow!("NSPasteboard setString returned NO"));
        }
    }
    Ok(())
}
