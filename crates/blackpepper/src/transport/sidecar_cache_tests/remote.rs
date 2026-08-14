use super::*;

#[cfg(unix)]
#[test]
fn declared_license_is_retained_and_uploaded_beside_the_binary() {
    let archive = compress(
        ArchiveKind::TarGz,
        &archive_with_files(&[("zellij", b"zellij-binary"), ("LICENSE.md", b"MIT license")]),
    );
    let base = synthetic_asset(ArchiveKind::TarGz, "zellij", &archive);
    let asset = Box::leak(Box::new(ReleaseAsset {
        license_name: Some("LICENSE.md"),
        ..base.clone()
    }));
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::at(temporary.path().join("cache"));
    let cached = cache.ensure(asset, &BytesDownloader::new(archive)).unwrap();

    let license = cached.license_path.as_ref().unwrap();
    assert_eq!(std::fs::read(license).unwrap(), b"MIT license");
    assert_eq!(cached.license_sha256.as_deref().unwrap().len(), 64);
    assert_private_mode(license, 0o600);

    let remote_home = temporary.path().join("remote-home");
    std::fs::create_dir(&remote_home).unwrap();
    let remote = install_remote(&mut LocalTransport, &cached, &remote_home).unwrap();
    let remote_license = remote.license_path.unwrap();
    assert_eq!(std::fs::read(remote_license).unwrap(), b"MIT license");
    assert_eq!(remote.license_sha256, cached.license_sha256);
}

#[cfg(unix)]
#[test]
fn verified_binary_upload_is_atomic_and_executable() {
    let archive = regular_archive(ArchiveKind::TarGz, "zellij", b"remote-zellij");
    let asset = synthetic_asset(ArchiveKind::TarGz, "zellij", &archive);
    let temporary = TempDir::new().unwrap();
    let cache = SidecarCache::at(temporary.path().join("cache"));
    let cached = cache.ensure(asset, &BytesDownloader::new(archive)).unwrap();
    let remote_home = temporary.path().join("remote-home");
    std::fs::create_dir(&remote_home).unwrap();

    let installed = install_remote(&mut LocalTransport, &cached, &remote_home).unwrap();
    assert_eq!(
        std::fs::read(&installed.binary_path).unwrap(),
        b"remote-zellij"
    );
    assert_eq!(installed.binary_sha256, cached.binary_sha256);
    assert_private_mode(&installed.binary_path, 0o700);
    assert_private_mode(&remote_home.join(".local/share/blackpepper"), 0o700);
    assert!(installed
        .binary_path
        .parent()
        .unwrap()
        .read_dir()
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".upload")));
}
