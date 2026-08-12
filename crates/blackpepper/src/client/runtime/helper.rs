use std::path::{Path, PathBuf};

const PRODUCTION_HELPER: &str = "bp-host";

/// Resolve the helper packaged beside the running client.
///
/// Development installs keep this sibling in a private, build-identified
/// bundle, so rebuilding `bp-dev` cannot replace the helper used by `bp`.
pub(super) fn sibling_helper_path() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let helper = sibling_helper_candidate(&executable);
    if helper.is_absolute() && helper.is_file() {
        Ok(helper)
    } else {
        Err(format!(
            "The matching {} helper was not installed beside {}.",
            helper
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(PRODUCTION_HELPER),
            executable.display(),
        ))
    }
}

fn sibling_helper_candidate(executable: &Path) -> PathBuf {
    executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(PRODUCTION_HELPER)
}

#[cfg(test)]
mod tests {
    use super::sibling_helper_candidate;
    use std::path::{Path, PathBuf};

    #[test]
    fn every_client_uses_the_helper_in_its_own_bundle() {
        assert_eq!(
            sibling_helper_candidate(Path::new("/opt/blackpepper/bp-dev")),
            PathBuf::from("/opt/blackpepper/bp-host")
        );
        assert_eq!(
            sibling_helper_candidate(Path::new("/opt/blackpepper/bp")),
            PathBuf::from("/opt/blackpepper/bp-host")
        );
    }
}
