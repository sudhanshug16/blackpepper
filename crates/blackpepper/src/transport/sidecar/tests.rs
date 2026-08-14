use super::*;

#[test]
fn manifest_has_trusted_assets_for_every_supported_target() {
    let mut identities = std::collections::BTreeSet::new();
    for asset in sidecar_manifest::assets() {
        assert!(identities.insert((asset.tool, asset.version, asset.target)));
        assert_eq!(asset.checksum().unwrap().len(), 64);
        assert!(asset.url.starts_with("https://github.com/"));
        assert!(asset.url.ends_with(asset.asset_name));
        assert_eq!(asset.license_name.is_some(), asset.license_sha256.is_some());
        if is_blackpepper_zellij_version(asset.version) {
            assert_eq!(asset.binary_sha256.unwrap().len(), 64);
            assert_eq!(asset.license_name, Some("LICENSES.html"));
        }
    }
    assert_eq!(sidecar_manifest::assets().len(), 8);
}

#[test]
fn historical_zellij_assets_remain_addressable_by_recorded_version() {
    let asset = release_asset_for_version(
        ManagedTool::Zellij,
        LEGACY_ZELLIJ_VERSION,
        SidecarTarget::MacOsAarch64,
    )
    .unwrap();

    assert_eq!(asset.version, LEGACY_ZELLIJ_VERSION);
    assert!(asset.url.contains("zellij-org/zellij/releases"));
}

#[test]
fn branded_zellij_is_always_a_managed_runtime() {
    assert!(is_blackpepper_zellij_version(PATCHED_ZELLIJ_VERSION));
    assert!(is_blackpepper_zellij_version("0.44.3-blackpepper.7"));
    assert!(is_blackpepper_zellij_version("0.45.0-blackpepper.1"));
    assert!(!is_blackpepper_zellij_version(LEGACY_ZELLIJ_VERSION));
    assert!(!is_blackpepper_zellij_version("0.44.3-blackpepper."));
    assert!(!is_blackpepper_zellij_version("0.44.3-blackpepper.dev"));
    let installed = SystemRuntime {
        binary: PathBuf::from("/usr/bin/zellij"),
        version: format!("zellij {PATCHED_ZELLIJ_VERSION}"),
    };

    // The private manifest is deliberately absent until its artifacts have
    // been published. Reaching lookup instead of accepting PATH proves the
    // system executable cannot satisfy the branded runtime.
    assert!(matches!(
        select_runtime_for_version(
            ManagedTool::Zellij,
            PATCHED_ZELLIJ_VERSION,
            SidecarTarget::LinuxX86_64,
            Some(installed),
        ),
        Err(SidecarError::UnsupportedAsset { .. })
    ));
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
