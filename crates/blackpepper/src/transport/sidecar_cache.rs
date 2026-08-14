use std::path::{Path, PathBuf};

use super::sidecar::{sha256_file, ReleaseAsset, SidecarError, VerifiedArchive};
use super::sidecar_cache_fs::{create_private_dir, open_lock, set_file_mode};
use super::sidecar_download::SidecarDownloader;
use super::sidecar_upload::UploadPlan;

mod install;
mod license;

use install::{cached_file_is_valid, ensure_archive, install_binary};
use license::install_license;

/// Private, versioned cache for release archives and extracted executables.
#[derive(Debug, Clone)]
pub struct SidecarCache {
    root: PathBuf,
    application_root: Option<PathBuf>,
}

impl SidecarCache {
    /// Resolve `<XDG data>/blackpepper/sidecars` on this client.
    pub fn from_xdg() -> Result<Self, SidecarError> {
        Ok(Self::under_data_home(local_data_home()?))
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
        if !cached_file_is_valid(&binary_path, &digest_path)? {
            install_binary(asset, &archive_path, &binary_path, &digest_path, &directory)?;
        } else {
            set_file_mode(&binary_path, 0o700)?;
            set_file_mode(&digest_path, 0o600)?;
        }
        let binary_sha256 = sha256_file(&binary_path)?;
        if let Some(expected) = asset.binary_sha256 {
            if !binary_sha256.eq_ignore_ascii_case(expected) {
                return Err(SidecarError::ChecksumMismatch {
                    asset: asset.binary_name.to_owned(),
                    expected: expected.to_owned(),
                    actual: binary_sha256,
                });
            }
        }
        let (license_path, license_sha256) = if let Some(license_name) = asset.license_name {
            if Path::new(license_name)
                .file_name()
                .and_then(|name| name.to_str())
                != Some(license_name)
            {
                return Err(SidecarError::InvalidArchive {
                    asset: asset.asset_name.to_owned(),
                    message: "declares an unsafe license filename".to_owned(),
                });
            }
            let path = directory.join(license_name);
            let digest = directory.join(format!(".{license_name}.sha256"));
            if !cached_file_is_valid(&path, &digest)? {
                install_license(
                    asset,
                    &archive_path,
                    license_name,
                    &path,
                    &digest,
                    &directory,
                )?;
            } else {
                set_file_mode(&path, 0o600)?;
                set_file_mode(&digest, 0o600)?;
            }
            let sha256 = sha256_file(&path)?;
            if let Some(expected) = asset.license_sha256 {
                if !sha256.eq_ignore_ascii_case(expected) {
                    return Err(SidecarError::ChecksumMismatch {
                        asset: license_name.to_owned(),
                        expected: expected.to_owned(),
                        actual: sha256,
                    });
                }
            }
            (Some(path), Some(sha256))
        } else {
            (None, None)
        };

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
            license_path,
            license_sha256,
        })
    }
}

/// Resolve the one local data root used by both the client and `bp-host`.
pub fn local_data_home() -> Result<PathBuf, SidecarError> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(SidecarError::UnsafeCacheEntry {
                path,
                message: "XDG_DATA_HOME must be absolute".to_string(),
            });
        }
        return Ok(path);
    }
    dirs::data_dir().ok_or_else(|| SidecarError::UnsafeCacheEntry {
        path: PathBuf::new(),
        message: "could not resolve the client data directory".to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct CachedSidecar {
    asset: &'static ReleaseAsset,
    verified_archive: VerifiedArchive,
    pub archive_path: PathBuf,
    pub binary_path: PathBuf,
    pub binary_sha256: String,
    pub license_path: Option<PathBuf>,
    pub license_sha256: Option<String>,
}

impl CachedSidecar {
    pub fn asset(&self) -> &'static ReleaseAsset {
        self.asset
    }

    pub fn upload_plan(&self, remote_home: &Path) -> Result<UploadPlan, SidecarError> {
        UploadPlan::new_with_license(
            self.verified_archive,
            &self.binary_path,
            &self.binary_sha256,
            self.license_path.as_deref(),
            self.license_sha256.as_deref(),
            remote_home,
        )
    }

    pub fn upload_plan_in_data_home(
        &self,
        remote_data_home: &Path,
    ) -> Result<UploadPlan, SidecarError> {
        UploadPlan::new_with_license_in_data_home(
            self.verified_archive,
            &self.binary_path,
            &self.binary_sha256,
            self.license_path.as_deref(),
            self.license_sha256.as_deref(),
            remote_data_home,
        )
    }
}
