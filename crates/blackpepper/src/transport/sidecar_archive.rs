use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use tar::{Archive, EntryType};
use xz2::read::XzDecoder;

use super::sidecar::{ArchiveKind, ReleaseAsset, SidecarError};

const MAX_BINARY_BYTES: u64 = 256 * 1024 * 1024;

/// Extract exactly the named executable from an already verified archive.
///
/// No archive path is ever joined to a filesystem path. This keeps skipped
/// documentation files harmless and makes traversal entries fail closed.
pub(crate) fn extract_binary(
    asset: &ReleaseAsset,
    archive_path: &Path,
    output: &mut File,
) -> Result<(), SidecarError> {
    let archive = File::open(archive_path).map_err(|source| SidecarError::Io {
        operation: format!("failed to open verified archive {}", archive_path.display()),
        source,
    })?;

    match asset.archive {
        ArchiveKind::TarGz => scan_archive(asset, Archive::new(GzDecoder::new(archive)), output),
        ArchiveKind::TarXz => scan_archive(asset, Archive::new(XzDecoder::new(archive)), output),
    }
}

fn scan_archive<R: Read>(
    asset: &ReleaseAsset,
    mut archive: Archive<R>,
    output: &mut File,
) -> Result<(), SidecarError> {
    let entries = archive.entries().map_err(|error| invalid(asset, error))?;
    let mut found = false;

    for entry in entries {
        let mut entry = entry.map_err(|error| invalid(asset, error))?;
        let path = entry
            .path()
            .map_err(|error| invalid(asset, error))?
            .into_owned();
        validate_archive_path(asset, &path)?;

        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err(SidecarError::InvalidArchive {
                asset: asset.asset_name.to_string(),
                message: format!(
                    "{} is an unsupported {} entry",
                    path.display(),
                    entry_type_name(entry_type)
                ),
            });
        }
        if path.file_name() != Some(OsStr::new(asset.binary_name)) {
            continue;
        }
        if found {
            return Err(SidecarError::InvalidArchive {
                asset: asset.asset_name.to_string(),
                message: format!("contains more than one {} binary", asset.binary_name),
            });
        }

        let declared_size = entry
            .header()
            .size()
            .map_err(|error| invalid(asset, error))?;
        if declared_size > MAX_BINARY_BYTES {
            return Err(SidecarError::InvalidArchive {
                asset: asset.asset_name.to_string(),
                message: format!(
                    "{} is larger than the {}-byte binary limit",
                    path.display(),
                    MAX_BINARY_BYTES
                ),
            });
        }
        let copied = io::copy(&mut entry.by_ref().take(MAX_BINARY_BYTES + 1), output)
            .map_err(|error| invalid(asset, error))?;
        if copied > MAX_BINARY_BYTES || copied != declared_size {
            return Err(SidecarError::InvalidArchive {
                asset: asset.asset_name.to_string(),
                message: format!("{} has an invalid or oversized payload", path.display()),
            });
        }
        found = true;
    }

    if !found {
        return Err(SidecarError::InvalidArchive {
            asset: asset.asset_name.to_string(),
            message: format!("does not contain the expected {} binary", asset.binary_name),
        });
    }
    output.flush().map_err(|source| SidecarError::Io {
        operation: "failed to flush extracted sidecar".to_string(),
        source,
    })?;
    output.sync_all().map_err(|source| SidecarError::Io {
        operation: "failed to sync extracted sidecar".to_string(),
        source,
    })
}

fn validate_archive_path(asset: &ReleaseAsset, path: &Path) -> Result<(), SidecarError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(SidecarError::InvalidArchive {
            asset: asset.asset_name.to_string(),
            message: format!("contains unsafe path {}", path.display()),
        });
    }
    Ok(())
}

fn entry_type_name(entry_type: EntryType) -> &'static str {
    if entry_type.is_symlink() {
        "symbolic-link"
    } else if entry_type.is_hard_link() {
        "hard-link"
    } else {
        "non-regular"
    }
}

fn invalid(asset: &ReleaseAsset, error: impl std::fmt::Display) -> SidecarError {
    SidecarError::InvalidArchive {
        asset: asset.asset_name.to_string(),
        message: error.to_string(),
    }
}
