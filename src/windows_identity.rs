use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL, LUID};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenStatistics, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_STATISTICS, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

#[derive(Clone)]
pub(crate) struct WindowsSessionIdentity {
    user_sid: String,
    logon_luid: LUID,
}

impl WindowsSessionIdentity {
    pub(crate) fn current() -> Result<Self> {
        let token = current_process_token()?;
        let user_sid = token_user_sid_string(token.raw())?;
        let statistics: TOKEN_STATISTICS = token_information(token.raw(), TokenStatistics)?;
        Ok(Self {
            user_sid,
            logon_luid: statistics.AuthenticationId,
        })
    }

    pub(crate) fn user_sid(&self) -> &str {
        &self.user_sid
    }

    pub(crate) fn scoped_name_suffix(&self) -> String {
        scoped_name_suffix(&self.user_sid, self.logon_luid)
    }
}

pub(crate) struct SecurityAttributes {
    attrs: SECURITY_ATTRIBUTES,
    descriptor: PSECURITY_DESCRIPTOR,
}

impl SecurityAttributes {
    pub(crate) fn for_current_user_ipc(identity: &WindowsSessionIdentity) -> Result<Self> {
        let sddl = ipc_security_sddl(identity.user_sid());
        let mut descriptor = std::ptr::null_mut();
        let wide = wide_null(&sddl);
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| "convert IPC security descriptor");
        }
        Ok(Self {
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.cast(),
                bInheritHandle: 0,
            },
            descriptor,
        })
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut std::ffi::c_void {
        (&mut self.attrs as *mut SECURITY_ATTRIBUTES).cast()
    }
}

impl Drop for SecurityAttributes {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            unsafe {
                let _ = LocalFree(self.descriptor as HLOCAL);
            }
        }
    }
}

fn current_process_token() -> Result<Handle> {
    let mut token = std::ptr::null_mut();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("open current process token");
    }
    Ok(Handle(token))
}

fn token_information<T: Copy>(token: HANDLE, class: i32) -> Result<T> {
    let mut len = 0;
    unsafe {
        let _ = GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut len);
    }
    if len == 0 {
        return Err(std::io::Error::last_os_error()).context("query token information size");
    }
    let mut buffer = vec![0u8; len as usize];
    let ok =
        unsafe { GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("query token information");
    }
    debug_assert!(buffer.len() >= std::mem::size_of::<T>());
    Ok(unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<T>()) })
}

fn token_user_sid_string(token: HANDLE) -> Result<String> {
    let mut len = 0;
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
    }
    if len == 0 {
        return Err(std::io::Error::last_os_error()).context("query token user size");
    }
    let mut buffer = vec![0u8; len as usize];
    let ok =
        unsafe { GetTokenInformation(token, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("query token user");
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    sid_to_string(user.User.Sid).context("convert current user SID")
}

fn sid_to_string(sid: *mut std::ffi::c_void) -> Result<String> {
    let mut raw = std::ptr::null_mut();
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut raw) };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("convert SID to string");
    }
    let value = wide_ptr_to_string(raw);
    unsafe {
        let _ = LocalFree(raw as HLOCAL);
    }
    Ok(value)
}

fn wide_ptr_to_string(ptr: *const u16) -> String {
    let mut len = 0usize;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
    }
}

fn scoped_name_suffix(user_sid: &str, logon_luid: LUID) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_sid.as_bytes());
    hasher.update(b":");
    hasher.update(logon_luid.HighPart.to_le_bytes());
    hasher.update(logon_luid.LowPart.to_le_bytes());
    let digest = hasher.finalize();
    digest[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ipc_security_sddl(user_sid: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{user_sid})")
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(HANDLE);

impl Handle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_name_suffix_is_stable_and_not_raw_sid() {
        let luid = LUID {
            LowPart: 0x9abc_def0,
            HighPart: 0x1234_5678,
        };
        let sid = "S-1-5-21-1000-2000-3000-1001";
        let first = scoped_name_suffix(sid, luid);
        let second = scoped_name_suffix(sid, luid);

        assert_eq!(first, second);
        assert_eq!(first.len(), 24);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!first.contains("S-1-5"));
    }

    #[test]
    fn security_sddl_restricts_to_current_user_system_and_admins() {
        let sddl = ipc_security_sddl("S-1-5-21-1000-2000-3000-1001");

        assert_eq!(
            sddl,
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;S-1-5-21-1000-2000-3000-1001)"
        );
        assert!(!sddl.contains("WD"));
        assert!(!sddl.contains("AN"));
    }
}
