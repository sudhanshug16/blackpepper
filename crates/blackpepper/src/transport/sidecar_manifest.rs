use super::sidecar::{ArchiveKind, ManagedTool, ReleaseAsset, SidecarError, SidecarTarget};

// Archive digests are pinned from each repository's immutable GitHub release
// metadata. Runtime code never downloads a sibling checksum file.
const ASSETS: &[ReleaseAsset] = &[
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::LinuxX86_64,
        version: "0.44.3",
        asset_name: "zellij-x86_64-unknown-linux-musl.tar.gz",
        url: "https://github.com/zellij-org/zellij/releases/download/v0.44.3/zellij-x86_64-unknown-linux-musl.tar.gz",
        trusted_sha256: Some(
            "0f7c346788627f506c0a28296517768633cff24fc822a739f8264b640ecad751",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::LinuxAarch64,
        version: "0.44.3",
        asset_name: "zellij-aarch64-unknown-linux-musl.tar.gz",
        url: "https://github.com/zellij-org/zellij/releases/download/v0.44.3/zellij-aarch64-unknown-linux-musl.tar.gz",
        trusted_sha256: Some(
            "15e6534d42644d66973d136c590c49739dcfd6a1a2a0d3d917973f16c81b45fb",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::MacOsX86_64,
        version: "0.44.3",
        asset_name: "zellij-x86_64-apple-darwin.tar.gz",
        url: "https://github.com/zellij-org/zellij/releases/download/v0.44.3/zellij-x86_64-apple-darwin.tar.gz",
        trusted_sha256: Some(
            "59f803faa32cd4e5f316f0dc2d3b7a5530a72553e38ad939286471848a418eeb",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::MacOsAarch64,
        version: "0.44.3",
        asset_name: "zellij-aarch64-apple-darwin.tar.gz",
        url: "https://github.com/zellij-org/zellij/releases/download/v0.44.3/zellij-aarch64-apple-darwin.tar.gz",
        trusted_sha256: Some(
            "b6acf83a7739cf5f0f4e9bd47709642d4d98acbbf8c34d4a12c6e706f531da61",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
    },
    ReleaseAsset {
        tool: ManagedTool::Worktrunk,
        target: SidecarTarget::LinuxX86_64,
        version: "0.72.0",
        asset_name: "worktrunk-x86_64-unknown-linux-musl.tar.xz",
        url: "https://github.com/max-sixty/worktrunk/releases/download/v0.72.0/worktrunk-x86_64-unknown-linux-musl.tar.xz",
        trusted_sha256: Some(
            "e91bc7ceb0623942a797317f56541a825d6a36e24d055985a8299d30345be346",
        ),
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
    },
    ReleaseAsset {
        tool: ManagedTool::Worktrunk,
        target: SidecarTarget::LinuxAarch64,
        version: "0.72.0",
        asset_name: "worktrunk-aarch64-unknown-linux-musl.tar.xz",
        url: "https://github.com/max-sixty/worktrunk/releases/download/v0.72.0/worktrunk-aarch64-unknown-linux-musl.tar.xz",
        trusted_sha256: Some(
            "2f6b45fd0592e4b0f66ca3c34cbaf90c7643a7eaabf8a9c4b0e12d48251a086c",
        ),
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
    },
    ReleaseAsset {
        tool: ManagedTool::Worktrunk,
        target: SidecarTarget::MacOsX86_64,
        version: "0.72.0",
        asset_name: "worktrunk-x86_64-apple-darwin.tar.xz",
        url: "https://github.com/max-sixty/worktrunk/releases/download/v0.72.0/worktrunk-x86_64-apple-darwin.tar.xz",
        trusted_sha256: Some(
            "2356bee43a6688a03d24b27dd18ce0db1f4666f111ee06f3c829d1f248472401",
        ),
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
    },
    ReleaseAsset {
        tool: ManagedTool::Worktrunk,
        target: SidecarTarget::MacOsAarch64,
        version: "0.72.0",
        asset_name: "worktrunk-aarch64-apple-darwin.tar.xz",
        url: "https://github.com/max-sixty/worktrunk/releases/download/v0.72.0/worktrunk-aarch64-apple-darwin.tar.xz",
        trusted_sha256: Some(
            "7e6cf79a3ef67559240431aae93c137d9a2b28a8ccdb55b64edead904b21ff73",
        ),
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
    },
];

pub(crate) fn release_asset(
    tool: ManagedTool,
    target: SidecarTarget,
) -> Result<&'static ReleaseAsset, SidecarError> {
    let asset = ASSETS
        .iter()
        .find(|asset| asset.tool == tool && asset.target == target)
        .ok_or(SidecarError::UnsupportedAsset { tool, target })?;
    asset.checksum()?;
    Ok(asset)
}

#[cfg(test)]
pub(crate) fn assets() -> &'static [ReleaseAsset] {
    ASSETS
}
