use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use tempfile::Builder as TempBuilder;

use super::SidecarError;

pub(super) struct SizeLimitedWriter<'a> {
    inner: &'a mut dyn Write,
    limit: u64,
    written: u64,
    exceeded: bool,
}

impl<'a> SizeLimitedWriter<'a> {
    pub(super) fn new(inner: &'a mut dyn Write, limit: u64) -> Self {
        Self {
            inner,
            limit,
            written: 0,
            exceeded: false,
        }
    }

    pub(super) fn exceeded(&self) -> bool {
        self.exceeded
    }
}

impl Write for SizeLimitedWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.written.saturating_add(buffer.len() as u64) > self.limit {
            self.exceeded = true;
            return Err(io::Error::other("managed sidecar archive is too large"));
        }
        let written = self.inner.write(buffer)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

pub(super) fn atomic_write(
    path: &Path,
    contents: &[u8],
    directory: &Path,
) -> Result<(), SidecarError> {
    let mut temporary = TempBuilder::new()
        .prefix(".metadata-")
        .tempfile_in(directory)
        .map_err(io_error("failed to create sidecar metadata"))?;
    temporary
        .write_all(contents)
        .map_err(io_error("failed to write sidecar metadata"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(io_error("failed to sync sidecar metadata"))?;
    set_file_mode(temporary.path(), 0o600)?;
    atomic_replace(temporary, path)
}

pub(super) fn atomic_replace(
    file: tempfile::NamedTempFile,
    destination: &Path,
) -> Result<(), SidecarError> {
    fs::rename(file.path(), destination).map_err(io_error("failed to publish sidecar cache file"))
}

pub(super) fn create_private_dir(path: &Path) -> Result<(), SidecarError> {
    fs::create_dir_all(path).map_err(io_error("failed to create sidecar cache directory"))?;
    let metadata =
        fs::symlink_metadata(path).map_err(io_error("failed to inspect sidecar cache"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(SidecarError::UnsafeCacheEntry {
            path: path.to_path_buf(),
            message: "expected a real directory".to_string(),
        });
    }
    set_file_mode(path, 0o700)
}

pub(super) fn open_lock(path: &Path) -> Result<File, SidecarError> {
    reject_unsafe_existing(path)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(io_error("failed to open sidecar cache lock"))?;
    set_file_mode(path, 0o600)?;
    Ok(file)
}

pub(super) fn reject_unsafe_existing(path: &Path) -> Result<(), SidecarError> {
    let _ = regular_file(path)?;
    Ok(())
}

pub(super) fn regular_file(path: &Path) -> Result<Option<fs::Metadata>, SidecarError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata))
        }
        Ok(_) => Err(SidecarError::UnsafeCacheEntry {
            path: path.to_path_buf(),
            message: "expected a regular file".to_string(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(SidecarError::Io {
            operation: format!("failed to inspect {}", path.display()),
            source,
        }),
    }
}

#[cfg(unix)]
pub(super) fn set_file_mode(path: &Path, mode: u32) -> Result<(), SidecarError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(io_error("failed to secure sidecar cache entry"))
}

#[cfg(not(unix))]
pub(super) fn set_file_mode(_path: &Path, _mode: u32) -> Result<(), SidecarError> {
    Ok(())
}

pub(super) fn sync_directory(path: &Path) -> Result<(), SidecarError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(io_error("failed to sync sidecar cache directory"))
}

pub(super) fn io_error(operation: &'static str) -> impl FnOnce(io::Error) -> SidecarError {
    move |source| SidecarError::Io {
        operation: operation.to_string(),
        source,
    }
}
