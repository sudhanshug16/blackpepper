use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::sidecar_manifest;

mod error;
pub use error::SidecarError;

/// The private, checksum-pinned release used by new sessions.
pub const ZELLIJ_VERSION: &str = "0.44.3-blackpepper.2";
pub const LEGACY_ZELLIJ_VERSION: &str = "0.44.3";
pub const PATCHED_ZELLIJ_VERSION: &str = "0.44.3-blackpepper.2";
pub const WORKTRUNK_VERSION: &str = "0.72.0";

/// Private Zellij builds must never be satisfied by an executable from PATH.
pub fn is_blackpepper_zellij_version(version: &str) -> bool {
    let Some((upstream, generation)) = version.rsplit_once("-blackpepper.") else {
        return false;
    };
    !generation.is_empty()
        && generation.bytes().all(|byte| byte.is_ascii_digit())
        && upstream.split('.').all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ManagedTool {
    Zellij,
    Worktrunk,
}

impl ManagedTool {
    pub fn version(self) -> &'static str {
        match self {
            Self::Zellij => ZELLIJ_VERSION,
            Self::Worktrunk => WORKTRUNK_VERSION,
        }
    }

    pub fn binary_name(self) -> &'static str {
        match self {
            Self::Zellij => "zellij",
            Self::Worktrunk => "wt",
        }
    }
}

impl fmt::Display for ManagedTool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Zellij => "zellij",
            Self::Worktrunk => "worktrunk",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SidecarTarget {
    LinuxX86_64,
    LinuxAarch64,
    MacOsX86_64,
    MacOsAarch64,
}

impl SidecarTarget {
    pub fn current() -> Result<Self, SidecarError> {
        Self::from_os_arch(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub fn from_uname(os: &str, architecture: &str) -> Result<Self, SidecarError> {
        let os = os.trim().to_ascii_lowercase();
        let os = match os.as_str() {
            "darwin" => "macos",
            other => other,
        };
        let architecture = architecture.trim().to_ascii_lowercase();
        let architecture = match architecture.as_str() {
            "amd64" => "x86_64",
            "arm64" => "aarch64",
            other => other,
        };
        Self::from_os_arch(os, architecture)
    }

    pub fn from_os_arch(os: &str, architecture: &str) -> Result<Self, SidecarError> {
        match (os, architecture) {
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("linux", "aarch64") => Ok(Self::LinuxAarch64),
            ("macos", "x86_64") => Ok(Self::MacOsX86_64),
            ("macos", "aarch64") => Ok(Self::MacOsAarch64),
            _ => Err(SidecarError::UnsupportedPlatform {
                os: os.to_string(),
                architecture: architecture.to_string(),
            }),
        }
    }

    pub fn triple(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "x86_64-unknown-linux-musl",
            Self::LinuxAarch64 => "aarch64-unknown-linux-musl",
            Self::MacOsX86_64 => "x86_64-apple-darwin",
            Self::MacOsAarch64 => "aarch64-apple-darwin",
        }
    }

    pub fn is_linux(self) -> bool {
        matches!(self, Self::LinuxX86_64 | Self::LinuxAarch64)
    }
}

impl fmt::Display for SidecarTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.triple())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    TarXz,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub tool: ManagedTool,
    pub target: SidecarTarget,
    pub version: &'static str,
    pub asset_name: &'static str,
    pub url: &'static str,
    pub trusted_sha256: Option<&'static str>,
    /// SHA-256 of the extracted executable, pinned independently of the archive.
    pub binary_sha256: Option<&'static str>,
    pub archive: ArchiveKind,
    pub binary_name: &'static str,
    /// Optional license file distributed beside a Blackpepper-owned binary.
    pub license_name: Option<&'static str>,
    /// SHA-256 of `license_name` when a license is declared.
    pub license_sha256: Option<&'static str>,
}

impl ReleaseAsset {
    pub fn checksum(&self) -> Result<&'static str, SidecarError> {
        self.trusted_sha256.ok_or(SidecarError::NoTrustedChecksum {
            tool: self.tool,
            target: self.target,
        })
    }

    pub fn verify(&'static self, bytes: &[u8]) -> Result<VerifiedArchive, SidecarError> {
        let expected = self.checksum()?;
        let actual = sha256_bytes(bytes);
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(SidecarError::ChecksumMismatch {
                asset: self.asset_name.to_string(),
                expected: expected.to_string(),
                actual,
            });
        }
        Ok(VerifiedArchive { asset: self })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VerifiedArchive {
    asset: &'static ReleaseAsset,
}

impl VerifiedArchive {
    pub fn asset(self) -> &'static ReleaseAsset {
        self.asset
    }
}

pub fn release_asset(
    tool: ManagedTool,
    target: SidecarTarget,
) -> Result<&'static ReleaseAsset, SidecarError> {
    release_asset_for_version(tool, tool.version(), target)
}

pub fn release_asset_for_version(
    tool: ManagedTool,
    version: &str,
    target: SidecarTarget,
) -> Result<&'static ReleaseAsset, SidecarError> {
    sidecar_manifest::release_asset(tool, version, target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemRuntime {
    pub binary: PathBuf,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSelection {
    System(SystemRuntime),
    Managed(&'static ReleaseAsset),
}

/// Prefer a system binary only when its protocol/CLI version exactly matches.
pub fn select_runtime(
    tool: ManagedTool,
    target: SidecarTarget,
    installed: Option<SystemRuntime>,
) -> Result<RuntimeSelection, SidecarError> {
    select_runtime_for_version(tool, tool.version(), target, installed)
}

pub fn select_runtime_for_version(
    tool: ManagedTool,
    version: &str,
    target: SidecarTarget,
    installed: Option<SystemRuntime>,
) -> Result<RuntimeSelection, SidecarError> {
    let requires_managed = tool == ManagedTool::Zellij && is_blackpepper_zellij_version(version);
    if let Some(installed) = installed {
        let installed_version = installed
            .version
            .split_whitespace()
            .last()
            .unwrap_or_default()
            .trim_start_matches('v');
        if !requires_managed
            && !installed.binary.as_os_str().is_empty()
            && installed_version == version
        {
            return Ok(RuntimeSelection::System(installed));
        }
    }
    Ok(RuntimeSelection::Managed(release_asset_for_version(
        tool, version, target,
    )?))
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String, SidecarError> {
    let mut file = File::open(path).map_err(|source| SidecarError::Io {
        operation: format!("failed to open {}", path.display()),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| SidecarError::Io {
            operation: format!("failed to read {}", path.display()),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

impl ReleaseAsset {
    /// Verify an archive already written to disk before any decoder sees it.
    pub fn verify_file(&'static self, path: &Path) -> Result<VerifiedArchive, SidecarError> {
        let expected = self.checksum()?;
        let actual = sha256_file(path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(SidecarError::ChecksumMismatch {
                asset: self.asset_name.to_string(),
                expected: expected.to_string(),
                actual,
            });
        }
        Ok(VerifiedArchive { asset: self })
    }
}

#[cfg(test)]
mod tests;
