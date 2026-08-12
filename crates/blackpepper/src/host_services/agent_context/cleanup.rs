use super::{now_millis, AgentContextStore};
use crate::agent_status::Provider;
use crate::core::AgentRunId;
use rusqlite::{params, TransactionBehavior};

impl AgentContextStore {
    pub(super) fn cleanup_abandoned_unbound(&mut self, cutoff: i64) -> Result<(), String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id, provider FROM agent_run_context
                 WHERE active = 1 AND session_id IS NULL AND created_at_ms < ?1",
            )
            .map_err(|error| error.to_string())?;
        let stale = statement
            .query_map([cutoff], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);

        // Make abandoned launch descriptors undiscoverable before touching
        // their best-effort managed assets. An undeletable file must not keep
        // one stale launch active and break every later host recovery scan.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        for (run_id, _) in &stale {
            transaction
                .execute(
                    "UPDATE agent_run_context
                     SET active = 0, deactivated_at_ms = ?2
                     WHERE run_id = ?1 AND active = 1 AND session_id IS NULL",
                    params![run_id, now_millis() as i64],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;

        let integration_dir = self
            .path
            .parent()
            .ok_or_else(|| "Agent context database has no state directory.".to_owned())?
            .join("integrations");
        let mut cleanup_errors = Vec::new();
        for (run_id, provider) in stale {
            let Ok(run_id) = run_id.parse::<AgentRunId>() else {
                cleanup_errors.push("Stored abandoned run ID is invalid.".to_owned());
                continue;
            };
            let Ok(provider) = provider.parse::<Provider>() else {
                cleanup_errors.push(format!(
                    "Stored provider for abandoned run {run_id} is invalid."
                ));
                continue;
            };
            let file = match provider {
                Provider::Codex => None,
                Provider::Claude => Some(integration_dir.join(format!("claude-{run_id}.json"))),
                Provider::OpenCode => Some(integration_dir.join(format!("opencode-{run_id}.js"))),
            };
            if let Some(file) = file {
                match std::fs::remove_file(&file) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        cleanup_errors.push(format!(
                            "Could not clean abandoned integration {}: {error}",
                            file.display()
                        ));
                    }
                }
            }
        }
        if cleanup_errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Abandoned agent launches were deactivated, but managed integration cleanup failed: {}",
                cleanup_errors.join("; ")
            ))
        }
    }
}
