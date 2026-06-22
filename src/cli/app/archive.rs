use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub fn verify_sha256(bytes: &[u8], checksum_body: &str) -> Result<()> {
    let expected = checksum_body
        .split_whitespace()
        .next()
        .context("checksum file is empty")?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        anyhow::bail!("checksum mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

pub fn extract_shuo_binary(tar_gz: &[u8], out_dir: &Path, expected_path: &Path) -> Result<PathBuf> {
    fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    let decoder = flate2::read::GzDecoder::new(Cursor::new(tar_gz));
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().context("read release archive entries")? {
        let mut entry = entry.context("read release archive entry")?;
        let path = entry
            .path()
            .context("read archive entry path")?
            .to_path_buf();
        if !is_safe_relative_path(&path) {
            continue;
        }
        if path != expected_path {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let target = out_dir.join("shuo");
        entry
            .unpack(&target)
            .with_context(|| format!("extract shuo binary to {}", target.display()))?;
        return Ok(target);
    }

    anyhow::bail!(
        "release archive does not contain expected shuo binary at {}",
        expected_path.display()
    )
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn verifies_matching_sha256() {
        let bytes = b"abc";
        verify_sha256(
            bytes,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
    }

    #[test]
    fn verifies_standard_shasum_file_format() {
        let bytes = b"abc";
        verify_sha256(
            bytes,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  shuo.tar.gz\n",
        )
        .unwrap();
    }

    #[test]
    fn rejects_mismatched_sha256() {
        let err = verify_sha256(b"abc", "0000").unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err:#}");
    }

    fn tar_gz_with_file(path: &str, body: &[u8]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, path, body).unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn tar_gz_with_dir(path: &str) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Directory);
            header.set_size(0);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, path, std::io::empty())
                .unwrap();
            builder.finish().unwrap();
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn extracts_shuo_binary_from_release_directory() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-extract-{}", ulid::Ulid::new()));
        let archive = tar_gz_with_file("shuo-v0.2.0-aarch64-apple-darwin/shuo", b"binary");

        let extracted = extract_shuo_binary(
            &archive,
            &dir,
            Path::new("shuo-v0.2.0-aarch64-apple-darwin/shuo"),
        )
        .unwrap();

        assert_eq!(fs::read(extracted).unwrap(), b"binary");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_shuo_at_unexpected_safe_path() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-extract-{}", ulid::Ulid::new()));
        let archive = tar_gz_with_file("docs/shuo", b"wrong");

        let err = extract_shuo_binary(
            &archive,
            &dir,
            Path::new("shuo-v0.2.0-aarch64-apple-darwin/shuo"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("expected shuo binary"), "{err:#}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_non_file_at_expected_path() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-extract-{}", ulid::Ulid::new()));
        let archive = tar_gz_with_dir("shuo-v0.2.0-aarch64-apple-darwin/shuo");

        let err = extract_shuo_binary(
            &archive,
            &dir,
            Path::new("shuo-v0.2.0-aarch64-apple-darwin/shuo"),
        )
        .unwrap_err();

        assert!(err.to_string().contains("expected shuo binary"), "{err:#}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_archive_without_shuo() {
        let dir =
            std::env::temp_dir().join(format!("shuohua-update-extract-{}", ulid::Ulid::new()));
        let archive = tar_gz_with_file("README.md", b"readme");

        let err = extract_shuo_binary(
            &archive,
            &dir,
            Path::new("shuo-v0.2.0-aarch64-apple-darwin/shuo"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected shuo binary"), "{err:#}");
        let _ = fs::remove_dir_all(dir);
    }
}
