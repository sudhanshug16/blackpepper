use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tempfile::Builder as TempBuilder;

use super::sidecar::{sha256_file, ReleaseAsset, SidecarError, VerifiedArchive};
use super::sidecar_archive::extract_binary;
use super::sidecar_cache_fs::{
    atomic_replace, atomic_write, create_private_dir, io_error, open_lock, regular_file,
    reject_unsafe_existing, set_file_mode, sync_directory, SizeLimitedWriter,
};
use super::sidecar_download::{SidecarDownloader, MAX_ARCHIVE_BYTES};
use super::sidecar_upload::UploadPlan;

/// Private, versioned cache for release archives and extracted executables.
#[derive(Debug, Clone)]
pub struct SidecarCache {
    root: PathBuf,
    application_root: Option<PathBuf>,
}

impl SidecarCache {
    /// Resolve `<XDG data>/blackpepper/sidecars` on this client.
    pub fn from_xdg() -> Result<Self, SidecarError> {
        let data_home = match std::env::var_os("XDG_DATA_HOME") {
            Some(path) => {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    return Err(SidecarError::UnsafeCacheEntry {
                        path,
                        message: "XDG_DATA_HOME must be absolute".to_string(),
                    });
                }
                path
            }
            None => dirs::data_dir().ok_or_else(|| SidecarError::UnsafeCacheEntry {
                path: PathBuf::new(),
                message: "could not resolve the client data directory".to_string(),
            })?,
        };
        Ok(Self::under_data_home(data_home))
    }

    pub fn under_data_home(data_home: impl Into<PathBuf>) -> Self {
        let application_root = data_home.into().join("blackpepper");
        Self {
            root: application_root.join("sidecars"),
            application_root: Some(application_root),
        }
    }

    /// Build a cache at an exact root, primarily for isolated runtimes/tests.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            application_root: None,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn ensure(
        &self,
        asset: &'static ReleaseAsset,
        downloader: &dyn SidecarDownloader,
    ) -> Result<CachedSidecar, SidecarError> {
        let directory = self
            .root
            .join(asset.tool.to_string())
            .join(asset.version)
            .join(asset.target.triple());
        if let Some(application_root) = &self.application_root {
            create_private_dir(application_root)?;
        }
        create_private_dir(&self.root)?;
        create_private_dir(&self.root.join(asset.tool.to_string()))?;
        create_private_dir(&self.root.join(asset.tool.to_string()).join(asset.version))?;
        create_private_dir(&directory)?;

        let lock = open_lock(&directory.join(".install.lock"))?;
        fs2::FileExt::lock_exclusive(&lock).map_err(|source| SidecarError::Io {
            operation: "failed to lock the managed sidecar cache".to_string(),
            source,
        })?;

        let archive_path = directory.join(asset.asset_name);
        let verified_archive = ensure_archive(asset, &archive_path, &directory, downloader)?;
        let binary_path = directory.join(asset.binary_name);
        let digest_path = directory.join(format!(".{}.sha256", asset.binary_name));
        if !cached_binary_is_valid(&binary_path, &digest_path)? {
            install_binary(asset, &archive_path, &binary_path, &digest_path, &directory)?;
        } else {
            set_file_mode(&binary_path, 0o700)?;
            set_file_mode(&digest_path, 0o600)?;
        }
        let binary_sha256 = sha256_file(&binary_path)?;

        fs2::FileExt::unlock(&lock).map_err(|source| SidecarError::Io {
            operation: "failed to unlock the managed sidecar cache".to_string(),
            source,
        })?;
        Ok(CachedSidecar {
            asset,
            verified_archive,
            archive_path,
            binary_path,
            binary_sha256,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CachedSidecar {
    asset: &'static ReleaseAsset,
    verified_archive: VerifiedArchive,
    pub archive_path: PathBuf,
    pub binary_path: PathBuf,
    pub binary_sha256: String,
}

impl CachedSidecar {
    pub fn asset(&self) -> &'static ReleaseAsset {
        self.asset
    }

    pub fn upload_plan(&self, remote_home: &Path) -> Result<UploadPlan, SidecarError> {
        UploadPlan::new(
            self.verified_archive,
            &self.binary_path,
            &self.binary_sha256,
            remote_home,
        )
    }

    pub fn upload_plan_in_data_home(
        &self,
        remote_data_home: &Path,
    ) -> Result<UploadPlan, SidecarError> {
        UploadPlan::new_in_data_home(
            self.verified_archive,
            &self.binary_path,
            &self.binary_sha256,
            remote_data_home,
        )
    }
}

fn ensure_archive(
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

fn install_binary(
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

fn cached_binary_is_valid(binary: &Path, digest: &Path) -> Result<bool, SidecarError> {
    let Some(binary_metadata) = regular_file(binary)? else {
        reject_unsafe_existing(digest)?;
        return Ok(false);
    };
    let Some(digest_metadata) = regular_file(digest)? else {
        return Ok(false);
    };
    if binary_metadata.len() == 0 || digest_metadata.len() > 128 {
        return Ok(false);
    }
    let expected =
        fs::read_to_string(digest).map_err(io_error("failed to read sidecar receipt"))?;
    let expected = expected.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(false);
    }
    Ok(sha256_file(binary)?.eq_ignore_ascii_case(expected))
}
