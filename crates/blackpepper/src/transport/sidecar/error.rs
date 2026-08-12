use super::{ManagedTool, SidecarTarget};
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum SidecarError {
    UnsupportedPlatform {
        os: String,
        architecture: String,
    },
    UnsupportedAsset {
        tool: ManagedTool,
        target: SidecarTarget,
    },
    NoTrustedChecksum {
        tool: ManagedTool,
        target: SidecarTarget,
    },
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    Download {
        url: String,
        message: String,
    },
    ArchiveTooLarge {
        asset: String,
        limit: u64,
    },
    InvalidArchive {
        asset: String,
        message: String,
    },
    UnsafeCacheEntry {
        path: PathBuf,
        message: String,
    },
    InvalidUploadPlan(String),
    Io {
        operation: String,
        source: io::Error,
    },
}

impl fmt::Display for SidecarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform { os, architecture } => {
                write!(
                    formatter,
                    "unsupported sidecar platform: {os}/{architecture}"
                )
            }
            Self::UnsupportedAsset { tool, target } => {
                write!(
                    formatter,
                    "no managed {tool} asset is available for {target}"
                )
            }
            Self::NoTrustedChecksum { tool, target } => write!(
                formatter,
                "managed {tool} for {target} has no embedded trusted checksum"
            ),
            Self::ChecksumMismatch {
                asset,
                expected,
                actual,
            } => write!(
                formatter,
                "checksum mismatch for {asset}: expected {expected}, got {actual}"
            ),
            Self::Download { url, message } => {
                write!(formatter, "failed to download {url}: {message}")
            }
            Self::ArchiveTooLarge { asset, limit } => write!(
                formatter,
                "managed sidecar archive {asset} exceeds the {limit}-byte limit"
            ),
            Self::InvalidArchive { asset, message } => {
                write!(
                    formatter,
                    "invalid managed sidecar archive {asset}: {message}"
                )
            }
            Self::UnsafeCacheEntry { path, message } => {
                write!(
                    formatter,
                    "unsafe sidecar cache entry {}: {message}",
                    path.display()
                )
            }
            Self::InvalidUploadPlan(message) => formatter.write_str(message),
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for SidecarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
