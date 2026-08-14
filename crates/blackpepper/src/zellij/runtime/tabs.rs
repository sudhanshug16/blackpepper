use std::path::Path;
use std::time::Duration;

use crate::transport::{HostCommand, HostTransport, TransportError};

use super::super::model::{checked, ZellijError};
use super::validation::{path_text, validate_name, ValidateInitialCommand, BACKGROUND_TAB_LAYOUT};
use super::ZellijRuntime;

mod close;
mod focus;
mod preflight;
mod reconcile;

use preflight::{BackgroundTabPreflight, BACKGROUND_PREFLIGHT_TIMEOUT};
use reconcile::{CreationReceipt, TAB_CREATION_RECONCILE_TIMEOUT};

// Cancellation is briefly masked from new-tab through focus compensation, so
// each action needs its own hard bound. A timeout is an unknown remote result;
// `ensure_tab` reconciles it without repeating the mutation.
const BACKGROUND_MUTATION_TIMEOUT: Duration = Duration::from_secs(5);

impl ZellijRuntime {
    pub fn new_tab_command(
        &self,
        session: &str,
        name: &str,
        cwd: &Path,
        initial_command: Option<&HostCommand>,
    ) -> Result<HostCommand, ZellijError> {
        validate_name("session", session)?;
        validate_name("tab", name)?;
        let cwd = path_text(cwd)?;
        // Zellij accepts `focus=false`, but 0.44.3 still moves its last-active
        // client to a dynamically created tab. `ensure_tab` therefore refuses
        // multiple clients and restores the sole client's previous tab.
        let mut command = self.session_action(
            session,
            [
                "new-tab",
                "--layout-string",
                BACKGROUND_TAB_LAYOUT,
                "--name",
                name,
                "--cwd",
                cwd.as_str(),
            ],
        );
        if let Some(initial) = initial_command {
            initial
                .validate_for_zellij()
                .map_err(ZellijError::InvalidName)?;
            command.args.push("--".to_string());
            command.args.push(initial.program.clone());
            command.args.extend(initial.args.iter().cloned());
        }
        Ok(command)
    }

    /// Return the exact tab ID and whether Zellij's own numeric response
    /// confirmed that this invocation created it. A pre-existing tab or a tab
    /// recovered from an empty/timeout response returns `false`, so callers do
    /// not destructively clean it up until their launch marker proves ownership.
    pub fn ensure_tab(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        name: &str,
        cwd: &Path,
        initial_command: Option<&HostCommand>,
    ) -> Result<(u64, bool), ZellijError> {
        self.ensure_tab_with_reconcile_timeout(
            host,
            session,
            name,
            cwd,
            initial_command,
            TAB_CREATION_RECONCILE_TIMEOUT,
        )
    }

    pub(crate) fn ensure_tab_with_reconcile_timeout(
        &self,
        host: &mut dyn HostTransport,
        session: &str,
        name: &str,
        cwd: &Path,
        initial_command: Option<&HostCommand>,
        reconcile_timeout: Duration,
    ) -> Result<(u64, bool), ZellijError> {
        let (preexisting_tab_ids, restore_focus) = match self.background_tab_preflight(
            host,
            session,
            name,
            BACKGROUND_PREFLIGHT_TIMEOUT,
        )? {
            BackgroundTabPreflight::Existing(tab_id) => return Ok((tab_id, false)),
            BackgroundTabPreflight::Create {
                preexisting_tab_ids,
                restore_focus,
            } => (preexisting_tab_ids, restore_focus),
        };
        let command = self.new_tab_command(session, name, cwd, initial_command)?;
        if crate::transport::CommandCancellation::scope_is_cancelled() {
            return Err(ZellijError::InvalidOutput(
                "background tab creation was cancelled after preflight; no new-tab was sent"
                    .to_string(),
            ));
        }
        crate::transport::CommandCancellation::mask_current(|| {
            let mutation_result = (|| {
                let receipt = match host.exec_timeout(&command, BACKGROUND_MUTATION_TIMEOUT) {
                    Ok(output) => {
                        let output = checked(output, "create Zellij tab")?;
                        if output.stdout.iter().all(u8::is_ascii_whitespace)
                            && output.stderr.is_empty()
                        {
                            CreationReceipt::unknown("new-tab returned no tab ID")
                        } else {
                            let tab_id = std::str::from_utf8(&output.stdout)
                                .ok()
                                .map(|value| {
                                    value.trim_matches(|character: char| {
                                        character.is_ascii_whitespace()
                                    })
                                })
                                .and_then(|value| value.parse::<u64>().ok())
                                .ok_or_else(|| {
                                    ZellijError::InvalidOutput(format!(
                                        "new-tab returned {} nonnumeric stdout byte(s) and {} stderr byte(s); refusing to infer tab ownership",
                                        output.stdout.len(),
                                        output.stderr.len()
                                    ))
                                })?;
                            CreationReceipt::reported(tab_id)
                        }
                    }
                    Err(
                        error @ (TransportError::CommandTimedOut { .. }
                        | TransportError::CommandCancelled { .. }),
                    ) => CreationReceipt::unknown(error.to_string()),
                    Err(error) => return Err(error.into()),
                };
                self.reconcile_created_tab(
                    host,
                    session,
                    name,
                    &preexisting_tab_ids,
                    receipt,
                    reconcile_timeout,
                )
            })();
            let focus_result = self.restore_background_focus(
                host,
                session,
                name,
                &preexisting_tab_ids,
                restore_focus,
            );
            match (mutation_result, focus_result) {
                (Ok(result), Ok(())) => Ok(result),
                (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
                (Err(mutation_error), Err(focus_error)) => Err(ZellijError::InvalidOutput(
                    format!("{mutation_error}; focus restoration also failed: {focus_error}"),
                )),
            }
        })
    }
}
