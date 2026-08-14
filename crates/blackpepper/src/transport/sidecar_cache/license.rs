use std::path::Path;

use tempfile::Builder as TempBuilder;

use super::super::sidecar::{sha256_file, ReleaseAsset, SidecarError};
use super::super::sidecar_archive::extract_license;
use super::super::sidecar_cache_fs::{
    atomic_replace, atomic_write, io_error, reject_unsafe_existing, set_file_mode, sync_directory,
};

pub(super) fn install_license(
    asset: &ReleaseAsset,
    archive_path: &Path,
    license_name: &str,
    license_path: &Path,
    digest_path: &Path,
    directory: &Path,
) -> Result<(), SidecarError> {
    reject_unsafe_existing(license_path)?;
    reject_unsafe_existing(digest_path)?;
    let mut temporary = TempBuilder::new()
        .prefix(".license-")
        .tempfile_in(directory)
        .map_err(|source| SidecarError::Io {
            operation: "failed to create an extracted sidecar license file".to_string(),
            source,
        })?;
    extract_license(asset, archive_path, license_name, temporary.as_file_mut())?;
    set_file_mode(temporary.path(), 0o600)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("failed to sync sidecar license"))?;
    let digest = sha256_file(temporary.path())?;
    atomic_replace(temporary, license_path)?;
    atomic_write(digest_path, format!("{digest}\n").as_bytes(), directory)?;
    sync_directory(directory)
}
