use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use crate::transport::{HostCommand, HostTransport};

use super::super::super::model::{checked, ZellijError};
use super::super::{LAUNCHER_PROGRAM, METADATA_TIMEOUT};

const FIELD_COUNT: usize = 5;
const NAMESPACE_ENV_SCRIPT: &str = "bp_zellij_uid=$(id -u) || exit; printf '%s\\0%s\\0%s\\0%s\\0%s\\0' \"$bp_zellij_uid\" \"${TMPDIR:-}\" \"${XDG_RUNTIME_DIR:-}\" \"${BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR:-}\" \"${ZELLIJ_SOCKET_DIR:-}\"";

pub(super) struct NamespaceEnvironment {
    uid: String,
    temporary_directory: Option<String>,
    xdg_runtime_directory: Option<String>,
    development_override: Option<String>,
    inherited_socket_directory: Option<String>,
}

impl NamespaceEnvironment {
    pub(super) fn candidates(&self, development_build: bool) -> Result<Vec<String>, ZellijError> {
        if development_build {
            if let Some(override_directory) = &self.development_override {
                return Ok(vec![validated_directory(
                    "BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR",
                    override_directory,
                )?]);
            }
        }

        let mut candidates = Vec::new();
        let mut seen = BTreeSet::new();
        push_unique(&mut candidates, &mut seen, self.canonical_directory());
        if let Some(root) = valid_legacy_directory(&self.temporary_directory) {
            push_child(
                &mut candidates,
                &mut seen,
                &root,
                &format!("zellij-{}", self.uid),
            );
        }
        push_unique(
            &mut candidates,
            &mut seen,
            format!("/run/user/{}/zellij", self.uid),
        );
        if let Some(root) = valid_legacy_directory(&self.xdg_runtime_directory) {
            push_child(&mut candidates, &mut seen, &root, "zellij");
        }
        if let Some(directory) = valid_legacy_directory(&self.inherited_socket_directory) {
            push_unique(&mut candidates, &mut seen, directory);
        }
        Ok(candidates)
    }

    pub(super) fn creation_directory(
        &self,
        development_build: bool,
    ) -> Result<String, ZellijError> {
        if development_build {
            if let Some(override_directory) = &self.development_override {
                return validated_directory(
                    "BLACKPEPPER_E2E_ZELLIJ_SOCKET_DIR",
                    override_directory,
                );
            }
        }
        Ok(self.canonical_directory())
    }

    fn canonical_directory(&self) -> String {
        format!("/tmp/zellij-{}", self.uid)
    }
}

#[cfg(test)]
pub(in crate::zellij::runtime) fn candidate_directories_for_test(
    uid: &str,
    temporary: Option<&str>,
    xdg: Option<&str>,
    development_override: Option<&str>,
    inherited_socket: Option<&str>,
    development_build: bool,
) -> Result<Vec<String>, ZellijError> {
    environment_for_test(uid, temporary, xdg, development_override, inherited_socket)
        .candidates(development_build)
}

#[cfg(test)]
fn environment_for_test(
    uid: &str,
    temporary: Option<&str>,
    xdg: Option<&str>,
    development_override: Option<&str>,
    inherited_socket: Option<&str>,
) -> NamespaceEnvironment {
    NamespaceEnvironment {
        uid: uid.to_owned(),
        temporary_directory: temporary.map(str::to_owned),
        xdg_runtime_directory: xdg.map(str::to_owned),
        development_override: development_override.map(str::to_owned),
        inherited_socket_directory: inherited_socket.map(str::to_owned),
    }
}

pub(super) fn namespace_environment(
    host: &mut dyn HostTransport,
) -> Result<NamespaceEnvironment, ZellijError> {
    // This resolver intentionally inherits the host's normal PATH. It runs
    // before workspace configuration is loaded, so a NixOS/Coreutils `id`
    // remains discoverable without allowing `[workspace.env]` to influence it.
    let command = HostCommand::new(LAUNCHER_PROGRAM).args([
        "-c",
        NAMESPACE_ENV_SCRIPT,
        "blackpepper-zellij-namespace",
    ]);
    let output = checked(
        host.exec_timeout(&command, METADATA_TIMEOUT)?,
        "resolve Zellij socket namespace",
    )?;
    let fields = parse_namespace_fields(&output.stdout)?;
    let uid = text_field(fields[0], "Unix user ID")?;
    if uid.is_empty() || uid.len() > 20 || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ZellijError::InvalidOutput(
            "The host returned an invalid Unix user ID for Zellij.".to_owned(),
        ));
    }
    Ok(NamespaceEnvironment {
        uid: uid.to_owned(),
        temporary_directory: optional_text_field(fields[1]),
        xdg_runtime_directory: optional_text_field(fields[2]),
        // Production deliberately ignores this internal test-only value,
        // including malformed bytes or paths.
        development_override: if crate::IS_DEVELOPMENT_BUILD {
            optional_utf8_field(fields[3], "development Zellij socket override")?
        } else {
            None
        },
        inherited_socket_directory: optional_text_field(fields[4]),
    })
}

fn parse_namespace_fields(output: &[u8]) -> Result<[&[u8]; FIELD_COUNT], ZellijError> {
    let fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.len() != FIELD_COUNT + 1 || !fields[FIELD_COUNT].is_empty() {
        return Err(ZellijError::InvalidOutput(
            "The host returned invalid Zellij namespace metadata.".to_owned(),
        ));
    }
    fields[..FIELD_COUNT].try_into().map_err(|_| {
        ZellijError::InvalidOutput(
            "The host returned invalid Zellij namespace metadata.".to_owned(),
        )
    })
}

fn text_field<'a>(value: &'a [u8], label: &str) -> Result<&'a str, ZellijError> {
    std::str::from_utf8(value)
        .map_err(|_| ZellijError::InvalidOutput(format!("The host {label} was not valid UTF-8.")))
}

fn optional_text_field(value: &[u8]) -> Option<String> {
    std::str::from_utf8(value)
        .ok()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn optional_utf8_field(value: &[u8], label: &str) -> Result<Option<String>, ZellijError> {
    if value.is_empty() {
        return Ok(None);
    }
    text_field(value, label).map(|value| Some(value.to_owned()))
}

pub(super) fn validated_directory(label: &str, value: &str) -> Result<String, ZellijError> {
    normalize_absolute(value).ok_or_else(|| {
        ZellijError::InvalidOutput(format!(
            "The host {label} must be an absolute path without parent traversal."
        ))
    })
}

fn valid_legacy_directory(value: &Option<String>) -> Option<String> {
    value.as_deref().and_then(normalize_absolute)
}

fn normalize_absolute(value: &str) -> Option<String> {
    let path = Path::new(value);
    if !path.is_absolute() || value.contains(['\0', '\n', '\r']) {
        return None;
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(Path::new("/")),
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    normalized.to_str().map(str::to_owned)
}

fn push_child(values: &mut Vec<String>, seen: &mut BTreeSet<String>, root: &str, child: &str) {
    if let Some(value) = Path::new(root).join(child).to_str() {
        push_unique(values, seen, value.to_owned());
    }
}

fn push_unique(values: &mut Vec<String>, seen: &mut BTreeSet<String>, value: String) {
    if seen.insert(value.clone()) {
        values.push(value);
    }
}
