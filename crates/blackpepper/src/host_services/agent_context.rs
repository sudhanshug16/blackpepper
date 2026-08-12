use crate::agent_status::Provider;
use crate::core::{AgentRunBinding, AgentRunId, HostId, HostRegistry, PaneId, WorkspaceId};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::path::{Path, PathBuf};

mod cleanup;
mod initialization_lock;
mod storage;
use initialization_lock::lock_initialization;
use storage::{
    decode_record, initialize_schema, load_record, now_millis, row_to_record, validate_binding,
    validate_context,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AgentRunContext {
    pub host_id: HostId,
    pub workspace_id: WorkspaceId,
    pub run_id: AgentRunId,
    pub pane_id: Option<PaneId>,
    pub provider: Provider,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StoredAgentRunContext {
    pub context: AgentRunContext,
    pub binding: Option<AgentRunBinding>,
    pub active: bool,
}

const MAX_ACTIVE_RUNS: usize = 1_000;
const UNBOUND_GRACE_MS: u64 = 60_000;

pub(super) struct AgentContextStore {
    connection: Connection,
    path: PathBuf,
}

impl AgentContextStore {
    pub fn open(path: &Path) -> Result<Self, String> {
        let _initialization_lock = lock_initialization(path)?;
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|error| error.to_string())?;
        initialize_schema(&connection).map_err(|error| error.to_string())?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    pub fn register(
        &mut self,
        registry: &HostRegistry,
        context: AgentRunContext,
    ) -> Result<(), String> {
        validate_context(registry, context)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        if let Some(existing) = load_record(&transaction, context.run_id)? {
            if existing.context != context {
                return Err("Agent run ID is already bound to different host context.".to_owned());
            }
            if !existing.active {
                return Err("Agent run is stale and cannot be reactivated.".to_owned());
            }
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(());
        }
        if let Some(pane_id) = context.pane_id {
            let occupied = transaction
                .query_row(
                    "SELECT run_id FROM agent_run_context
                     WHERE workspace_id = ?1 AND pane_id = ?2 AND active = 1",
                    params![context.workspace_id.to_string(), pane_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if occupied.is_some() {
                return Err(
                    "Agent pane already has an active run; reconcile or exit it first.".to_owned(),
                );
            }
        }
        transaction
            .execute(
                "INSERT INTO agent_run_context
                   (run_id, host_id, workspace_id, pane_id, provider, active, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)",
                params![
                    context.run_id.to_string(),
                    context.host_id.to_string(),
                    context.workspace_id.to_string(),
                    context.pane_id.map(|id| id.to_string()),
                    context.provider.as_str(),
                    now_millis() as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn bind(
        &mut self,
        registry: &HostRegistry,
        context: AgentRunContext,
        binding: &AgentRunBinding,
    ) -> Result<(), String> {
        validate_context(registry, context)?;
        validate_binding(registry, context, binding)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let stored = load_record(&transaction, context.run_id)?
            .ok_or_else(|| "Agent run is not registered.".to_owned())?;
        if !stored.active {
            return Err("Agent run is stale and cannot be bound.".to_owned());
        }
        if stored.context != context {
            return Err("Agent run binding does not match the registered context.".to_owned());
        }
        if let Some(existing) = stored.binding {
            if existing == *binding {
                transaction.commit().map_err(|error| error.to_string())?;
                return Ok(());
            }
            return Err("Agent run already has a different Zellij binding.".to_owned());
        }
        let changed = transaction
            .execute(
                "UPDATE agent_run_context SET
                   session_id = ?2, session_name = ?3, zellij_version = ?4,
                   tab_id = ?5, tab_name = ?6, zellij_pane_id = ?7,
                   bound_at_ms = ?8
                 WHERE run_id = ?1 AND active = 1 AND session_id IS NULL",
                params![
                    context.run_id.to_string(),
                    binding.session_id.to_string(),
                    &binding.session_name,
                    &binding.zellij_version,
                    binding.tab_id as i64,
                    &binding.tab_name,
                    &binding.zellij_pane_id,
                    now_millis() as i64,
                ],
            )
            .map_err(|error| format!("Could not persist the Zellij agent binding: {error}"))?;
        if changed != 1 {
            return Err("Agent run changed while its Zellij binding was recorded.".to_owned());
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn deactivate(&mut self, context: AgentRunContext) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let stored = load_record(&transaction, context.run_id)?
            .ok_or_else(|| "Agent run is not registered.".to_owned())?;
        if stored.context != context {
            return Err("Agent run deactivation does not match the registered context.".to_owned());
        }
        if stored.active {
            transaction
                .execute(
                    "UPDATE agent_run_context
                     SET active = 0, deactivated_at_ms = ?2
                     WHERE run_id = ?1 AND active = 1",
                    params![context.run_id.to_string(), now_millis() as i64],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn active(&self, run_id: AgentRunId) -> Result<Option<AgentRunContext>, String> {
        Ok(load_record(&self.connection, run_id)?
            .filter(|stored| stored.active)
            .map(|stored| stored.context))
    }

    pub fn record(&self, run_id: AgentRunId) -> Result<Option<StoredAgentRunContext>, String> {
        load_record(&self.connection, run_id)
    }

    pub fn active_bound(
        &mut self,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<Vec<StoredAgentRunContext>, String> {
        let cutoff = now_millis().saturating_sub(UNBOUND_GRACE_MS) as i64;
        self.cleanup_abandoned_unbound(cutoff)?;
        let sql = if workspace_id.is_some() {
            "SELECT run_id, host_id, workspace_id, pane_id, provider, active,
                    session_id, session_name, zellij_version, tab_id, tab_name,
                    zellij_pane_id
             FROM agent_run_context
             WHERE active = 1 AND session_id IS NOT NULL AND workspace_id = ?1
             ORDER BY created_at_ms, run_id LIMIT ?2"
        } else {
            "SELECT run_id, host_id, workspace_id, pane_id, provider, active,
                    session_id, session_name, zellij_version, tab_id, tab_name,
                    zellij_pane_id
             FROM agent_run_context
             WHERE active = 1 AND session_id IS NOT NULL
             ORDER BY created_at_ms, run_id LIMIT ?1"
        };
        let mut statement = self
            .connection
            .prepare(sql)
            .map_err(|error| error.to_string())?;
        let limit = (MAX_ACTIVE_RUNS + 1) as i64;
        let encoded = if let Some(workspace_id) = workspace_id {
            statement
                .query_map(params![workspace_id.to_string(), limit], row_to_record)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        } else {
            statement
                .query_map([limit], row_to_record)
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?
        };
        drop(statement);
        let mut records = encoded
            .into_iter()
            .map(decode_record)
            .collect::<Result<Vec<_>, _>>()?;
        if records.len() > MAX_ACTIVE_RUNS {
            return Err(format!(
                "Host has more than {MAX_ACTIVE_RUNS} active agent runs; exit stale runs before listing them."
            ));
        }
        // A bound row is invalid without the stable pane UUID required by the
        // hook and blocker protocols. Deactivate it instead of rediscovering a
        // run that cannot reject cross-pane events.
        let invalid = records
            .iter()
            .filter(|record| record.context.pane_id.is_none())
            .map(|record| record.context)
            .collect::<Vec<_>>();
        for context in invalid {
            self.deactivate(context)?;
        }
        records.retain(|record| record.context.pane_id.is_some());
        Ok(records)
    }
}
