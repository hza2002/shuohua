use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    config_root: PathBuf,
    state_root: PathBuf,
    cache_root: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Self {
        imp::discover()
    }

    pub fn from_roots(config_root: PathBuf, state_root: PathBuf, cache_root: PathBuf) -> Self {
        Self {
            config_root,
            state_root,
            cache_root,
        }
    }

    #[cfg(test)]
    fn from_unix_env(
        xdg_config_home: Option<&str>,
        xdg_state_home: Option<&str>,
        home: Option<&str>,
    ) -> Self {
        imp::from_unix_env(xdg_config_home, xdg_state_home, None, home)
    }

    #[cfg(test)]
    fn from_windows_known_roots(roaming_app_data: PathBuf, local_app_data: PathBuf) -> Self {
        Self::windows_product_roots(roaming_app_data, local_app_data)
    }

    #[cfg_attr(not(any(test, target_os = "windows")), allow(dead_code))]
    fn windows_product_roots(roaming_app_data: PathBuf, local_app_data: PathBuf) -> Self {
        Self::from_roots(
            roaming_app_data.join("Shuohua"),
            local_app_data.join("Shuohua"),
            local_app_data.join("Shuohua").join("cache"),
        )
    }

    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    pub fn main_config(&self) -> PathBuf {
        self.config_root.join("config.toml")
    }

    pub fn profile_dir(&self) -> PathBuf {
        self.config_root.join("profile")
    }

    pub fn asr_provider(&self, provider: &str) -> PathBuf {
        self.config_root
            .join("asr")
            .join(format!("{provider}.toml"))
    }

    pub fn post_dir(&self) -> PathBuf {
        self.config_root.join("post")
    }

    pub fn history(&self) -> PathBuf {
        self.state_root.join("history")
    }

    pub fn audio(&self) -> PathBuf {
        self.state_root.join("audio")
    }

    pub fn logs(&self) -> PathBuf {
        self.state_root.join("logs")
    }

    pub fn traces(&self) -> PathBuf {
        self.state_root.join("traces")
    }

    pub fn cache(&self) -> PathBuf {
        self.cache_root.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDirs {
    root: PathBuf,
}

impl StateDirs {
    pub fn discover() -> Self {
        Self::from_app_paths(&AppPaths::discover())
    }

    pub fn from_app_paths(paths: &AppPaths) -> Self {
        Self::from_root(paths.state_root().to_path_buf())
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn history(&self) -> PathBuf {
        self.root.join("history")
    }

    pub fn audio(&self) -> PathBuf {
        self.root.join("audio")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    #[cfg_attr(not(feature = "dev"), allow(dead_code))]
    pub fn traces(&self) -> PathBuf {
        self.root.join("traces")
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use super::AppPaths;
    use std::path::PathBuf;

    pub(super) fn discover() -> AppPaths {
        from_unix_env(
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("XDG_STATE_HOME").ok().as_deref(),
            std::env::var("XDG_CACHE_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        )
    }

    pub(super) fn from_unix_env(
        xdg_config_home: Option<&str>,
        xdg_state_home: Option<&str>,
        xdg_cache_home: Option<&str>,
        home: Option<&str>,
    ) -> AppPaths {
        let home = home.unwrap_or_default();
        let config_home = xdg_config_home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".config"));
        let state_home = xdg_state_home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".local/state"));
        let cache_home = xdg_cache_home
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".cache"));
        AppPaths::from_roots(
            config_home.join("shuohua"),
            state_home.join("shuohua"),
            cache_home.join("shuohua"),
        )
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::AppPaths;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::{ptr, slice};

    use windows_sys::Win32::Foundation::S_OK;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    pub(super) fn discover() -> AppPaths {
        let roaming = known_folder_roaming_app_data()
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(r".\AppData\Roaming"));
        let local = known_folder_local_app_data()
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(r".\AppData\Local"));
        from_windows_known_roots(roaming, local)
    }

    pub(super) fn from_windows_known_roots(
        roaming_app_data: PathBuf,
        local_app_data: PathBuf,
    ) -> AppPaths {
        AppPaths::windows_product_roots(roaming_app_data, local_app_data)
    }

    fn known_folder_roaming_app_data() -> Option<PathBuf> {
        known_folder_path(&FOLDERID_RoamingAppData)
    }

    fn known_folder_local_app_data() -> Option<PathBuf> {
        known_folder_path(&FOLDERID_LocalAppData)
    }

    fn known_folder_path(id: *const windows_sys::core::GUID) -> Option<PathBuf> {
        unsafe {
            let mut raw = ptr::null_mut();
            let result =
                SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, ptr::null_mut(), &mut raw);
            if result != S_OK {
                CoTaskMemFree(raw.cast());
                return None;
            }
            let path = OsString::from_wide(slice::from_raw_parts(raw, wcslen(raw)));
            CoTaskMemFree(raw.cast());
            Some(PathBuf::from(path))
        }
    }

    unsafe extern "C" {
        fn wcslen(buf: *const u16) -> usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_paths_unix_keep_terminal_friendly_config_and_state_roots() {
        let paths =
            AppPaths::from_unix_env(Some("/tmp/config"), Some("/tmp/state"), Some("/tmp/home"));
        assert_eq!(paths.config_root(), Path::new("/tmp/config/shuohua"));
        assert_eq!(paths.state_root(), Path::new("/tmp/state/shuohua"));
        assert_eq!(
            paths.main_config(),
            PathBuf::from("/tmp/config/shuohua/config.toml")
        );
        assert_eq!(
            paths.profile_dir(),
            PathBuf::from("/tmp/config/shuohua/profile")
        );
        assert_eq!(
            paths.asr_provider("doubao"),
            PathBuf::from("/tmp/config/shuohua/asr/doubao.toml")
        );
        assert_eq!(paths.post_dir(), PathBuf::from("/tmp/config/shuohua/post"));
        assert_eq!(paths.history(), PathBuf::from("/tmp/state/shuohua/history"));
        assert_eq!(paths.audio(), PathBuf::from("/tmp/state/shuohua/audio"));
        assert_eq!(paths.logs(), PathBuf::from("/tmp/state/shuohua/logs"));
        assert_eq!(paths.traces(), PathBuf::from("/tmp/state/shuohua/traces"));
        assert_eq!(paths.cache(), PathBuf::from("/tmp/home/.cache/shuohua"));
    }

    #[test]
    fn app_paths_windows_use_product_data_roots_not_package_private_data() {
        let roaming = PathBuf::from(r"C:\Users\Alice\AppData\Roaming");
        let local = PathBuf::from(r"C:\Users\Alice\AppData\Local");
        let paths = AppPaths::from_windows_known_roots(roaming.clone(), local.clone());
        assert_eq!(paths.config_root(), roaming.join("Shuohua"));
        assert_eq!(paths.state_root(), local.join("Shuohua"));
        assert_eq!(
            paths.main_config(),
            roaming.join("Shuohua").join("config.toml")
        );
        assert_eq!(paths.history(), local.join("Shuohua").join("history"));
        assert_eq!(paths.cache(), local.join("Shuohua").join("cache"));
    }

    #[test]
    fn state_dirs_can_be_derived_from_app_paths_state_root() {
        let paths = AppPaths::from_roots(
            PathBuf::from("/tmp/config/shuohua"),
            PathBuf::from("/tmp/state/shuohua"),
            PathBuf::from("/tmp/cache/shuohua"),
        );
        let dirs = StateDirs::from_app_paths(&paths);
        assert_eq!(dirs.root(), Path::new("/tmp/state/shuohua"));
        assert_eq!(dirs.history(), PathBuf::from("/tmp/state/shuohua/history"));
    }

    #[test]
    fn state_subdirectories_share_one_root() {
        let root = PathBuf::from("/tmp/shuohua-state");
        let dirs = StateDirs::from_root(root.clone());
        assert_eq!(dirs.root(), root);
        assert_eq!(dirs.history(), root.join("history"));
        assert_eq!(dirs.audio(), root.join("audio"));
        assert_eq!(dirs.logs(), root.join("logs"));
        assert_eq!(dirs.traces(), root.join("traces"));
    }
}
