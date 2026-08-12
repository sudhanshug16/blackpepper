use std::path::Path;

use crate::transport::HostCommand;

use super::super::model::ZellijError;

pub(super) trait ValidateInitialCommand {
    fn validate_for_zellij(&self) -> Result<(), String>;
}

impl ValidateInitialCommand for HostCommand {
    fn validate_for_zellij(&self) -> Result<(), String> {
        if self.program.is_empty()
            || self.program.contains('\0')
            || self.args.iter().any(|value| value.contains('\0'))
        {
            Err("initial tab command contains an invalid value".to_string())
        } else if self.cwd.is_some() || !self.env.is_empty() {
            Err("initial tab commands cannot override cwd or environment".to_string())
        } else {
            Ok(())
        }
    }
}

// KDL sibling and block nodes on one line need explicit semicolons. Zellij
// 0.44.3 rejects the visually plausible `pane } }` form at runtime.
pub(super) const BACKGROUND_TAB_LAYOUT: &str = "layout { tab focus=false { pane; }; }";

pub(super) fn validate_name(kind: &str, value: &str) -> Result<(), ZellijError> {
    if value.is_empty() || value.len() > 64 || value.contains(['\0', '\n', '\r']) {
        return Err(ZellijError::InvalidName(format!(
            "Zellij {kind} name must contain 1-64 single-line characters"
        )));
    }
    Ok(())
}

pub(super) fn validate_pane_selector(value: &str) -> Result<(), ZellijError> {
    let number = value
        .strip_prefix("terminal_")
        .or_else(|| value.strip_prefix("plugin_"))
        .unwrap_or(value);
    if number.is_empty() || number.parse::<u32>().is_err() {
        return Err(ZellijError::InvalidName(
            "Zellij pane ID must be terminal_N, plugin_N, or a non-negative integer".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_typed_pane_selector(value: &str) -> Result<(), ZellijError> {
    let Some(number) = value
        .strip_prefix("terminal_")
        .or_else(|| value.strip_prefix("plugin_"))
    else {
        return Err(ZellijError::InvalidName(
            "pane observation requires an unambiguous terminal_N or plugin_N selector".to_string(),
        ));
    };
    if number.is_empty() || number.parse::<u32>().is_err() {
        return Err(ZellijError::InvalidName(
            "pane observation requires an unambiguous terminal_N or plugin_N selector".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn path_text(path: &Path) -> Result<String, ZellijError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| ZellijError::InvalidName("Zellij cwd must be valid UTF-8".to_string()))
}
