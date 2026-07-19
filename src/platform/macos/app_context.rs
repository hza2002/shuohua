use objc2_app_kit::NSWorkspace;

use crate::platform::AppContext;

pub fn frontmost_app() -> AppContext {
    let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return AppContext::default();
    };
    AppContext {
        bundle_id: app.bundleIdentifier().map(|s| s.to_string()),
        app_name: app.localizedName().map(|s| s.to_string()),
    }
}
