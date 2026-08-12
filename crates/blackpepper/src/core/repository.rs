use super::{HostId, RepositoryId};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, path::Path};
use uuid::Uuid;

/// Repository identity is transport-neutral for remotes and host-scoped for local-only repos.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RepositoryIdentity {
    Remote {
        canonical_url: String,
    },
    Local {
        host_id: HostId,
        git_common_dir: String,
    },
}

impl RepositoryIdentity {
    pub fn remote(remote_url: &str) -> Result<Self, RepositoryIdentityError> {
        canonicalize_remote_url(remote_url)
            .map(|canonical_url| Self::Remote { canonical_url })
            .ok_or(RepositoryIdentityError::InvalidRemote)
    }

    /// `git_common_dir` must be canonicalized before it crosses this boundary.
    pub fn local(
        host_id: HostId,
        git_common_dir: impl Into<String>,
    ) -> Result<Self, RepositoryIdentityError> {
        let git_common_dir = git_common_dir.into();
        if git_common_dir.is_empty() {
            return Err(RepositoryIdentityError::EmptyLocalPath);
        }
        if !Path::new(&git_common_dir).is_absolute() {
            return Err(RepositoryIdentityError::RelativeLocalPath);
        }
        Ok(Self::Local {
            host_id,
            git_common_dir,
        })
    }

    pub fn repository_id(&self) -> RepositoryId {
        let key = match self {
            Self::Remote { canonical_url } => format!("remote:{canonical_url}"),
            Self::Local {
                host_id,
                git_common_dir,
            } => format!("local:{host_id}:{git_common_dir}"),
        };
        RepositoryId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes()))
    }
}

/// Per-workspace override for repository grouping in the client UI.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", content = "repository_id", rename_all = "snake_case")]
pub enum GroupingPolicy {
    /// Group matching repository identities; leave non-repositories ungrouped.
    #[default]
    Automatic,
    /// Never include this workspace in a repository group.
    Ungrouped,
    /// Join a chosen group even when automatic identity detection is unavailable.
    Explicit(RepositoryId),
}

impl GroupingPolicy {
    pub fn resolve(self, identity: Option<&RepositoryIdentity>) -> Option<RepositoryId> {
        match self {
            Self::Automatic => identity.map(RepositoryIdentity::repository_id),
            Self::Ungrouped => None,
            Self::Explicit(repository_id) => Some(repository_id),
        }
    }
}

/// Removes credentials and transport details while retaining host, non-default port, and path.
/// This lets SSH and HTTPS remotes for the same repository group together without storing tokens.
pub fn canonicalize_remote_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('/') || value.starts_with("./") {
        return None;
    }

    let (scheme, authority, path) = if let Some((scheme, rest)) = value.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        (Some(scheme.to_ascii_lowercase()), authority, path)
    } else {
        let (authority, path) = value.split_once(':')?;
        // Avoid treating drive-letter and ordinary relative paths as SCP-style remotes.
        if authority.len() == 1 || authority.contains(['/', '\\']) {
            return None;
        }
        (Some("ssh".to_owned()), authority, path)
    };

    let authority = authority.rsplit('@').next()?.to_ascii_lowercase();
    let authority = strip_default_port(&authority, scheme.as_deref());
    let path = path.split(['?', '#']).next()?.trim_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    if authority.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{authority}/{path}"))
}

fn strip_default_port<'a>(authority: &'a str, scheme: Option<&str>) -> &'a str {
    let default_port = match scheme {
        Some("ssh") => "22",
        Some("http") => "80",
        Some("https") => "443",
        Some("git") => "9418",
        _ => return authority,
    };
    authority
        .strip_suffix(default_port)
        .and_then(|without_port| without_port.strip_suffix(':'))
        .unwrap_or(authority)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryIdentityError {
    InvalidRemote,
    EmptyLocalPath,
    RelativeLocalPath,
}

impl fmt::Display for RepositoryIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidRemote => "repository remote is not a supported network URL",
            Self::EmptyLocalPath => "Git common directory cannot be empty",
            Self::RelativeLocalPath => "Git common directory must be an absolute path",
        };
        formatter.write_str(message)
    }
}

impl Error for RepositoryIdentityError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_remote_ignores_transport_credentials_and_default_ports() {
        let expected = Some("github.com/acme/pepper".to_owned());
        assert_eq!(
            canonicalize_remote_url("git@GitHub.com:acme/pepper.git"),
            expected
        );
        assert_eq!(
            canonicalize_remote_url("https://token@github.com:443/acme/pepper.git/"),
            expected
        );
        assert_eq!(
            canonicalize_remote_url("ssh://git@github.com:22/acme/pepper"),
            expected
        );
    }

    #[test]
    fn local_repositories_are_scoped_to_a_host() {
        let first = RepositoryIdentity::local(HostId::new(), "/srv/repo/.git").unwrap();
        let second = RepositoryIdentity::local(HostId::new(), "/srv/repo/.git").unwrap();
        assert_ne!(first.repository_id(), second.repository_id());
    }

    #[test]
    fn grouping_supports_automatic_explicit_and_ungrouped_modes() {
        let identity = RepositoryIdentity::remote("https://github.com/acme/pepper.git").unwrap();
        let explicit = RepositoryId::new();
        assert_eq!(
            GroupingPolicy::Automatic.resolve(Some(&identity)),
            Some(identity.repository_id())
        );
        assert_eq!(
            GroupingPolicy::Explicit(explicit).resolve(None),
            Some(explicit)
        );
        assert_eq!(GroupingPolicy::Ungrouped.resolve(Some(&identity)), None);
    }
}
