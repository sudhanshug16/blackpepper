use crate::updater::{self, UpdateOutcome};

use super::CommandResult;

pub(super) fn update_command() -> CommandResult {
    CommandResult {
        ok: true,
        message: update_message(updater::force_update_sync()),
        data: None,
    }
}

fn update_message(outcome: UpdateOutcome) -> String {
    match outcome {
        UpdateOutcome::Started => {
            "Update started. Restart Blackpepper to use the new version.".to_string()
        }
        UpdateOutcome::Completed => {
            "Update completed. Restart Blackpepper to use the new version.".to_string()
        }
        UpdateOutcome::SkippedDev => {
            "Update skipped for dev builds. Use the installer for releases.".to_string()
        }
        UpdateOutcome::SkippedDisabled => {
            "Update disabled via BLACKPEPPER_DISABLE_UPDATE.".to_string()
        }
        UpdateOutcome::SkippedCooldown => {
            "Update skipped due to cooldown. Try again later.".to_string()
        }
        UpdateOutcome::FailedSpawn => "Failed to start updater.".to_string(),
        UpdateOutcome::FailedExit => "Update failed. Check your network and try again.".to_string(),
    }
}
