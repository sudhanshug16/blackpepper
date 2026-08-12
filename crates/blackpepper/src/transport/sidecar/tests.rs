use super::*;

#[test]
fn manifest_has_trusted_assets_for_every_supported_target() {
    for asset in sidecar_manifest::assets() {
        assert_eq!(asset.version, asset.tool.version());
        assert_eq!(asset.checksum().unwrap().len(), 64);
        assert!(asset.url.starts_with("https://github.com/"));
        assert!(asset.url.ends_with(asset.asset_name));
    }
    assert_eq!(sidecar_manifest::assets().len(), 8);
}

#[test]
fn installed_runtime_must_exactly_match_pinned_version() {
    let installed = SystemRuntime {
        binary: PathBuf::from("/usr/bin/zellij"),
        version: "zellij 0.44.2".to_string(),
    };
    let selected = select_runtime(
        ManagedTool::Zellij,
        SidecarTarget::LinuxX86_64,
        Some(installed),
    )
    .unwrap();
    assert!(matches!(selected, RuntimeSelection::Managed(_)));
}

#[test]
fn checksum_mismatch_fails_closed() {
    let asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxX86_64).unwrap();
    assert!(matches!(
        asset.verify(b"not the release"),
        Err(SidecarError::ChecksumMismatch { .. })
    ));
}

#[test]
fn asset_without_an_embedded_checksum_fails_closed() {
    let mut asset = release_asset(ManagedTool::Zellij, SidecarTarget::LinuxX86_64)
        .unwrap()
        .clone();
    asset.trusted_sha256 = None;
    assert!(matches!(
        asset.checksum(),
        Err(SidecarError::NoTrustedChecksum { .. })
    ));
}

#[test]
fn uname_aliases_map_to_release_targets() {
    assert_eq!(
        SidecarTarget::from_uname("Linux", "amd64").unwrap(),
        SidecarTarget::LinuxX86_64
    );
    assert_eq!(
        SidecarTarget::from_uname("Darwin", "arm64").unwrap(),
        SidecarTarget::MacOsAarch64
    );
}
