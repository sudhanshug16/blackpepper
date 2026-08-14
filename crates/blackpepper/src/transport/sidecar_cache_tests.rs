use std::io::{Cursor, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use flate2::write::GzEncoder;
use flate2::Compression;
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;
use xz2::write::XzEncoder;

use super::{
    install_remote, sha256_bytes, ArchiveKind, LocalTransport, ManagedTool, ReleaseAsset,
    SidecarCache, SidecarDownloader, SidecarError, SidecarTarget,
};

mod remote;

struct BytesDownloader {
    bytes: Vec<u8>,
    calls: AtomicUsize,
}

impl BytesDownloader {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            calls: AtomicUsize::new(0),
        }
    }
}

impl SidecarDownloader for BytesDownloader {
    fn download(&self, _url: &str, destination: &mut dyn Write) -> Result<(), SidecarError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        destination
            .write_all(&self.bytes)
            .map_err(|source| SidecarError::Io {
                operation: "test download".to_string(),
                source,
            })
    }
}

struct OfflineDownloader;

impl SidecarDownloader for OfflineDownloader {
    fn download(&self, _url: &str, _destination: &mut dyn Write) -> Result<(), SidecarError> {
        panic!("a valid cache must not access the network")
    }
}

#[test]
fn downloads_once_then_works_offline_and_repairs_modes() {
    let archive = regular_archive(ArchiveKind::TarGz, "zellij", b"zellij-binary");
    let asset = synthetic_asset(ArchiveKind::TarGz, "zellij", &archive);
    let downloader = BytesDownloader::new(archive);
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::under_data_home(temporary.path().join("data"));

    let installed = cache.ensure(asset, &downloader).unwrap();
    assert_eq!(downloader.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read(&installed.binary_path).unwrap(),
        b"zellij-binary"
    );
    assert_private_mode(&installed.binary_path, 0o700);
    assert_private_mode(&installed.archive_path, 0o600);
    let version_directory = installed.binary_path.parent().unwrap();
    assert_private_mode(&version_directory.join(".zellij.sha256"), 0o600);
    assert_private_mode(&version_directory.join(".install.lock"), 0o600);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &installed.binary_path,
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    let cached = cache.ensure(asset, &OfflineDownloader).unwrap();
    assert_eq!(cached.binary_sha256, installed.binary_sha256);
    assert_private_mode(&cached.binary_path, 0o700);

    let version_directory = cached.binary_path.parent().unwrap();
    assert_private_mode(version_directory, 0o700);
    assert_private_mode(cache.root(), 0o700);
    assert_private_mode(cache.root().parent().unwrap(), 0o700);
}

#[test]
fn checksum_failure_never_extracts_or_publishes() {
    let trusted = regular_archive(ArchiveKind::TarGz, "zellij", b"trusted");
    let asset = synthetic_asset(ArchiveKind::TarGz, "zellij", &trusted);
    let downloader = BytesDownloader::new(b"not the release archive".to_vec());
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::at(temporary.path().join("cache"));

    assert!(matches!(
        cache.ensure(asset, &downloader),
        Err(SidecarError::ChecksumMismatch { .. })
    ));
    let binary = cache
        .root()
        .join("zellij/test/x86_64-unknown-linux-musl/zellij");
    assert!(!binary.exists());
}

#[test]
fn tar_xz_nested_binary_is_supported_without_extracting_other_files() {
    let tar = archive_with_files(&[
        ("worktrunk-x86_64/README.md", b"documentation"),
        ("worktrunk-x86_64/wt", b"worktrunk-binary"),
    ]);
    let archive = compress(ArchiveKind::TarXz, &tar);
    let asset = synthetic_asset(ArchiveKind::TarXz, "wt", &archive);
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::at(temporary.path().join("cache"));

    let installed = cache.ensure(asset, &BytesDownloader::new(archive)).unwrap();
    assert_eq!(
        std::fs::read(installed.binary_path).unwrap(),
        b"worktrunk-binary"
    );
    assert!(!temporary.path().join("worktrunk-x86_64/README.md").exists());
}

#[test]
fn cache_can_prepare_a_declared_target_other_than_the_client() {
    let archive = regular_archive(ArchiveKind::TarGz, "zellij", b"macos-sidecar");
    let asset = synthetic_asset_for_target(
        ArchiveKind::TarGz,
        "zellij",
        SidecarTarget::MacOsAarch64,
        &archive,
    );
    let temporary = TempDir::new().unwrap();
    let installed = SidecarCache::at(temporary.path().join("cache"))
        .ensure(asset, &BytesDownloader::new(archive))
        .unwrap();

    assert!(installed
        .binary_path
        .to_string_lossy()
        .contains("aarch64-apple-darwin"));
}

#[test]
fn traversal_and_symlink_entries_fail_closed() {
    for tar in [traversal_tar(), symlink_tar()] {
        let archive = compress(ArchiveKind::TarGz, &tar);
        let asset = synthetic_asset(ArchiveKind::TarGz, "zellij", &archive);
        let temporary = TempDir::new().unwrap();
        let cache = SidecarCache::at(temporary.path().join("cache"));

        assert!(matches!(
            cache.ensure(asset, &BytesDownloader::new(archive)),
            Err(SidecarError::InvalidArchive { .. })
        ));
        assert!(!temporary.path().join("zellij").exists());
    }
}

#[cfg(unix)]
#[test]
fn cache_symlink_is_not_followed() {
    use std::os::unix::fs::symlink;

    let archive = regular_archive(ArchiveKind::TarGz, "zellij", b"binary");
    let asset = synthetic_asset(ArchiveKind::TarGz, "zellij", &archive);
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::at(temporary.path().join("cache"));
    let directory = cache.root().join("zellij/test/x86_64-unknown-linux-musl");
    std::fs::create_dir_all(&directory).unwrap();
    let outside = temporary.path().join("outside");
    std::fs::write(&outside, b"do not overwrite").unwrap();
    symlink(&outside, directory.join(asset.asset_name)).unwrap();

    assert!(matches!(
        cache.ensure(asset, &BytesDownloader::new(archive)),
        Err(SidecarError::UnsafeCacheEntry { .. })
    ));
    assert_eq!(std::fs::read(outside).unwrap(), b"do not overwrite");
}

fn regular_archive(kind: ArchiveKind, binary: &'static str, contents: &[u8]) -> Vec<u8> {
    compress(kind, &archive_with_files(&[(binary, contents)]))
}

fn archive_with_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(EntryType::Regular);
        header.set_cksum();
        builder.append(&header, Cursor::new(*contents)).unwrap();
    }
    builder.into_inner().unwrap()
}

fn traversal_tar() -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    let path = b"../zellij";
    header.as_mut_bytes()[..path.len()].copy_from_slice(path);
    header.set_size(6);
    header.set_mode(0o755);
    header.set_entry_type(EntryType::Regular);
    header.set_cksum();
    builder.append(&header, Cursor::new(b"unsafe")).unwrap();
    builder.into_inner().unwrap()
}

fn symlink_tar() -> Vec<u8> {
    let mut builder = Builder::new(Vec::new());
    let mut header = Header::new_gnu();
    header.set_path("zellij").unwrap();
    header.set_size(0);
    header.set_mode(0o777);
    header.set_entry_type(EntryType::Symlink);
    header.set_link_name("/tmp/elsewhere").unwrap();
    header.set_cksum();
    builder.append(&header, Cursor::new([])).unwrap();
    builder.into_inner().unwrap()
}

fn compress(kind: ArchiveKind, tar: &[u8]) -> Vec<u8> {
    match kind {
        ArchiveKind::TarGz => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(tar).unwrap();
            encoder.finish().unwrap()
        }
        ArchiveKind::TarXz => {
            let mut encoder = XzEncoder::new(Vec::new(), 6);
            encoder.write_all(tar).unwrap();
            encoder.finish().unwrap()
        }
    }
}

fn synthetic_asset(
    archive: ArchiveKind,
    binary_name: &'static str,
    bytes: &[u8],
) -> &'static ReleaseAsset {
    synthetic_asset_for_target(archive, binary_name, SidecarTarget::LinuxX86_64, bytes)
}

fn synthetic_asset_for_target(
    archive: ArchiveKind,
    binary_name: &'static str,
    target: SidecarTarget,
    bytes: &[u8],
) -> &'static ReleaseAsset {
    let checksum = Box::leak(sha256_bytes(bytes).into_boxed_str());
    Box::leak(Box::new(ReleaseAsset {
        tool: if binary_name == "wt" {
            ManagedTool::Worktrunk
        } else {
            ManagedTool::Zellij
        },
        target,
        version: "test",
        asset_name: "test-sidecar.tar",
        url: "https://example.invalid/test-sidecar.tar",
        trusted_sha256: Some(checksum),
        binary_sha256: None,
        archive,
        binary_name,
        license_name: None,
        license_sha256: None,
    }))
}

#[cfg(unix)]
fn assert_private_mode(path: &std::path::Path, expected: u32) {
    use std::os::unix::fs::PermissionsExt;
    let actual = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(actual, expected, "unexpected mode for {}", path.display());
}

#[cfg(not(unix))]
fn assert_private_mode(_path: &std::path::Path, _expected: u32) {}
