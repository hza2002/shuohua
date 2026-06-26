//! Windows ONNX Runtime provisioning for Silero VAD.
//!
//! The product contract is a single `shuo.exe`. Windows still needs the ONNX
//! Runtime DLL internally, so the executable embeds the DLL and extracts it to
//! the product cache before initializing `ort` with an explicit path.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

const ORT_VERSION: &str = "1.22.0";
const ORT_DLL_NAME: &str = "onnxruntime.dll";
const ORT_DLL_BYTES: &[u8] =
    include_bytes!("../../assets/windows/onnxruntime/1.22.0/onnxruntime.dll");
const ORT_DLL_SHA256: &str = "579b636403983254346a5c1d80bd28f1519cd1e284cd204f8d4ff41f8d711559";

static INIT: OnceLock<Result<(), String>> = OnceLock::new();

pub fn init() -> Result<()> {
    INIT.get_or_init(|| init_once().map_err(|err| format!("{err:#}")))
        .clone()
        .map_err(anyhow::Error::msg)
}

fn init_once() -> Result<()> {
    let dll_path = provision_to_default_cache()?;
    ort::init_from(dll_path.to_string_lossy()).commit()?;
    Ok(())
}

fn provision_to_default_cache() -> Result<PathBuf> {
    let cache_root = crate::paths::AppPaths::discover()
        .cache()
        .join("runtime")
        .join("onnxruntime")
        .join(ORT_VERSION)
        .join(short_hash());
    provision_to(&cache_root)
}

fn provision_to(cache_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache_root)
        .with_context(|| format!("create ONNX Runtime cache dir {}", cache_root.display()))?;
    let dll_path = cache_root.join(ORT_DLL_NAME);
    if should_write(&dll_path)? {
        let tmp_path = cache_root.join(format!("{ORT_DLL_NAME}.tmp"));
        fs::write(&tmp_path, ORT_DLL_BYTES)
            .with_context(|| format!("write ONNX Runtime temp DLL {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &dll_path).with_context(|| {
            format!(
                "install ONNX Runtime DLL from {} to {}",
                tmp_path.display(),
                dll_path.display()
            )
        })?;
    }
    Ok(dll_path)
}

fn should_write(path: &Path) -> Result<bool> {
    match fs::read(path) {
        Ok(existing) => Ok(hex_sha256(&existing) != ORT_DLL_SHA256),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).with_context(|| format!("read {}", path.display())),
    }
}

fn short_hash() -> &'static str {
    &ORT_DLL_SHA256[..12]
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_ort_hash_matches_expected() {
        assert_eq!(hex_sha256(ORT_DLL_BYTES), ORT_DLL_SHA256);
    }

    #[test]
    fn provision_writes_embedded_dll_under_versioned_cache() {
        let dir = std::env::temp_dir().join(format!("shuohua-ort-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let dll = provision_to(&dir.join(ORT_VERSION).join(short_hash())).unwrap();

        assert_eq!(dll.file_name().unwrap(), ORT_DLL_NAME);
        assert!(dll.starts_with(dir.join(ORT_VERSION).join(short_hash())));
        assert_eq!(hex_sha256(&fs::read(&dll).unwrap()), ORT_DLL_SHA256);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn provision_replaces_wrong_dll_contents() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-ort-replace-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ORT_DLL_NAME), b"wrong").unwrap();

        let dll = provision_to(&dir).unwrap();

        assert_eq!(hex_sha256(&fs::read(&dll).unwrap()), ORT_DLL_SHA256);

        let _ = fs::remove_dir_all(&dir);
    }
}
