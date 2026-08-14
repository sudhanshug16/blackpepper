use std::fs;
use std::io::Write;
use std::path::Path;

use tempfile::Builder as TempBuilder;

use super::super::sidecar::{sha256_file, ReleaseAsset, SidecarError, VerifiedArchive};
use super::super::sidecar_archive::extract_binary;
use super::super::sidecar_cache_fs::{
    atomic_replace, atomic_write, io_error, regular_file, reject_unsafe_existing, set_file_mode,
    sync_directory, SizeLimitedWriter,
};
use super::super::sidecar_download::{SidecarDownloader, MAX_ARCHIVE_BYTES};

pub(super) fn ensure_archive(
    asset: &'static ReleaseAsset,
    archive_path: &Path,
    directory: &Path,
    downloader: &dyn SidecarDownloader,
) -> Result<VerifiedArchive, SidecarError> {
    if regular_file(archive_path)?.is_some() {
        set_file_mode(archive_path, 0o600)?;
        return asset.verify_file(archive_path);
    }
    let mut temporary = TempBuilder::new()
        .prefix(".archive-")
        .tempfile_in(directory)
        .map_err(|source| SidecarError::Io {
            operation: "failed to create a sidecar download file".to_string(),
            source,
        })?;
    let mut limited = SizeLimitedWriter::new(temporary.as_file_mut(), MAX_ARCHIVE_BYTES);
    let download = downloader.download(asset.url, &mut limited);
    if limited.exceeded() {
        return Err(SidecarError::ArchiveTooLarge {
            asset: asset.asset_name.to_string(),
            limit: MAX_ARCHIVE_BYTES,
        });
    }
    download?;
    if temporary
        .as_file()
        .metadata()
        .map_err(io_error("failed to inspect download"))?
        .len()
        > MAX_ARCHIVE_BYTES
    {
        return Err(SidecarError::ArchiveTooLarge {
            asset: asset.asset_name.to_string(),
            limit: MAX_ARCHIVE_BYTES,
        });
    }
    temporary
        .as_file_mut()
        .flush()
        .map_err(io_error("failed to flush download"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("failed to sync download"))?;
    let verified = asset.verify_file(temporary.path())?;
    set_file_mode(temporary.path(), 0o600)?;
    atomic_replace(temporary, archive_path)?;
    sync_directory(directory)?;
    Ok(verified)
}

pub(super) fn install_binary(
    asset: &ReleaseAsset,
    archive_path: &Path,
    binary_path: &Path,
    digest_path: &Path,
    directory: &Path,
) -> Result<(), SidecarError> {
    reject_unsafe_existing(binary_path)?;
    reject_unsafe_existing(digest_path)?;
    let mut temporary = TempBuilder::new()
        .prefix(".binary-")
        .tempfile_in(directory)
        .map_err(|source| SidecarError::Io {
            operation: "failed to create an extracted sidecar file".to_string(),
            source,
        })?;
    extract_binary(asset, archive_path, temporary.as_file_mut())?;
    set_file_mode(temporary.path(), 0o700)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("failed to sync sidecar binary"))?;
    let digest = sha256_file(temporary.path())?;
    atomic_replace(temporary, binary_path)?;
    atomic_write(digest_path, format!("{digest}\n").as_bytes(), directory)?;
    sync_directory(directory)
}

pub(super) fn cached_file_is_valid(file: &Path, digest: &Path) -> Result<bool, SidecarError> {
    let Some(file_metadata) = regular_file(file)? else {
        reject_unsafe_existing(digest)?;
        return Ok(false);
    };
    let Some(digest_metadata) = regular_file(digest)? else {
        return Ok(false);
    };
    if file_metadata.len() == 0 || digest_metadata.len() > 128 {
        return Ok(false);
    }
    let expected =
        fs::read_to_string(digest).map_err(io_error("failed to read sidecar receipt"))?;
    let expected = expected.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(false);
    }
    Ok(sha256_file(file)?.eq_ignore_ascii_case(expected))
}
