use super::sidecar::{ArchiveKind, ManagedTool, ReleaseAsset, SidecarError, SidecarTarget};

// Archive digests are pinned from published release checksum records. Runtime
// code never downloads a sibling checksum file.
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
        binary_sha256: None,
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: None,
        license_sha256: None,
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::LinuxX86_64,
        version: "0.44.3-blackpepper.1",
        asset_name: "zellij-x86_64-unknown-linux-musl.tar.gz",
        url: "https://github.com/sudhanshug16/blackpepper/releases/download/zellij-v0.44.3-blackpepper.1/zellij-x86_64-unknown-linux-musl.tar.gz",
        trusted_sha256: Some(
            "2811baaa7b0ee0d2193aa3e7c9884ae1a449a4956fdf4a380614e856a21b528f",
        ),
        binary_sha256: Some(
            "ab08e3dd4eadef69619e69e34c92f569d3146da0435085a5650d792adf79483e",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: Some("LICENSES.html"),
        license_sha256: Some(
            "d419799cd4078334ba492e8aae6b691264f618b4ea6d8fb3c72ba8bb5cdc5776",
        ),
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::LinuxAarch64,
        version: "0.44.3-blackpepper.1",
        asset_name: "zellij-aarch64-unknown-linux-musl.tar.gz",
        url: "https://github.com/sudhanshug16/blackpepper/releases/download/zellij-v0.44.3-blackpepper.1/zellij-aarch64-unknown-linux-musl.tar.gz",
        trusted_sha256: Some(
            "503ce8f13b3960401450fd9119fa5c7fec0fc09da0e0002bd7c5a9fb7c121596",
        ),
        binary_sha256: Some(
            "ab06009884f2af4658ad98a35b9c7593bc6b1bf50e8058b43c77703a07f899ea",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: Some("LICENSES.html"),
        license_sha256: Some(
            "d419799cd4078334ba492e8aae6b691264f618b4ea6d8fb3c72ba8bb5cdc5776",
        ),
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::MacOsX86_64,
        version: "0.44.3-blackpepper.1",
        asset_name: "zellij-x86_64-apple-darwin.tar.gz",
        url: "https://github.com/sudhanshug16/blackpepper/releases/download/zellij-v0.44.3-blackpepper.1/zellij-x86_64-apple-darwin.tar.gz",
        trusted_sha256: Some(
            "e4583673d802ba7f7a755d2d498f4abffe346aefa24950794e81a43246b68086",
        ),
        binary_sha256: Some(
            "55092dd786a2013109a2028699c51b8923edf3de62f15379a41b80e8a4cd60eb",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: Some("LICENSES.html"),
        license_sha256: Some(
            "d419799cd4078334ba492e8aae6b691264f618b4ea6d8fb3c72ba8bb5cdc5776",
        ),
    },
    ReleaseAsset {
        tool: ManagedTool::Zellij,
        target: SidecarTarget::MacOsAarch64,
        version: "0.44.3-blackpepper.1",
        asset_name: "zellij-aarch64-apple-darwin.tar.gz",
        url: "https://github.com/sudhanshug16/blackpepper/releases/download/zellij-v0.44.3-blackpepper.1/zellij-aarch64-apple-darwin.tar.gz",
        trusted_sha256: Some(
            "d15543a529bd05f3249d6823739bb6f9f9da70eb257cabcac2f7709db62944c9",
        ),
        binary_sha256: Some(
            "b5df03771ffe3fbaf12f3aaeef06c143b372633d03d93b82d7af4a618b3242d1",
        ),
        archive: ArchiveKind::TarGz,
        binary_name: "zellij",
        license_name: Some("LICENSES.html"),
        license_sha256: Some(
            "d419799cd4078334ba492e8aae6b691264f618b4ea6d8fb3c72ba8bb5cdc5776",
        ),
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
        binary_sha256: None,
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
        license_name: None,
        license_sha256: None,
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
        binary_sha256: None,
        archive: ArchiveKind::TarXz,
        binary_name: "wt",
        license_name: None,
        license_sha256: None,
    },
];

pub(crate) fn release_asset(
    tool: ManagedTool,
    version: &str,
    target: SidecarTarget,
) -> Result<&'static ReleaseAsset, SidecarError> {
    let asset = ASSETS
        .iter()
        .find(|asset| asset.tool == tool && asset.version == version && asset.target == target)
        .ok_or(SidecarError::UnsupportedAsset { tool, target })?;
    asset.checksum()?;
    Ok(asset)
}

#[cfg(test)]
pub(crate) fn assets() -> &'static [ReleaseAsset] {
    ASSETS
}
