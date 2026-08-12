use super::{AgentRunContext, StoredAgentRunContext};
use crate::core::{AgentRunBinding, AgentRunId, HostRegistry, SessionBackend, SessionState};
use rusqlite::{Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn validate_context(
    registry: &HostRegistry,
    context: AgentRunContext,
) -> Result<(), String> {
    let local_host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    if context.host_id != local_host_id {
        return Err("Agent run host does not match this helper installation.".to_owned());
    }
    let workspace = registry
        .workspace(context.workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Agent run workspace is not registered.".to_owned())?;
    if workspace.host_id != local_host_id {
        return Err("Agent run workspace belongs to another host.".to_owned());
    }
    Ok(())
}

pub(super) fn initialize_schema(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE IF NOT EXISTS agent_run_context (
           run_id TEXT PRIMARY KEY NOT NULL,
           host_id TEXT NOT NULL,
           workspace_id TEXT NOT NULL,
           pane_id TEXT,
           provider TEXT NOT NULL,
           active INTEGER NOT NULL CHECK(active IN (0, 1)),
           created_at_ms INTEGER NOT NULL,
           session_id TEXT,
           session_name TEXT,
           zellij_version TEXT,
           tab_id INTEGER,
           tab_name TEXT,
           zellij_pane_id TEXT,
           bound_at_ms INTEGER,
           deactivated_at_ms INTEGER
         );
         CREATE UNIQUE INDEX IF NOT EXISTS active_agent_pane
           ON agent_run_context(workspace_id, pane_id)
           WHERE pane_id IS NOT NULL AND active = 1;
         COMMIT;",
    )?;
    // Older V1 development databases predate persisted bindings. Each ALTER
    // is idempotently guarded so an interrupted migration can resume safely.
    for (name, definition) in [
        ("session_id", "TEXT"),
        ("session_name", "TEXT"),
        ("zellij_version", "TEXT"),
        ("tab_id", "INTEGER"),
        ("tab_name", "TEXT"),
        ("zellij_pane_id", "TEXT"),
        ("bound_at_ms", "INTEGER"),
        ("deactivated_at_ms", "INTEGER"),
    ] {
        if !column_exists(connection, name)? {
            connection.execute(
                &format!("ALTER TABLE agent_run_context ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    connection.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS active_agent_zellij_pane
           ON agent_run_context(session_id, zellij_pane_id)
           WHERE session_id IS NOT NULL AND zellij_pane_id IS NOT NULL AND active = 1;",
    )?;
    Ok(())
}

pub(super) fn load_record(
    connection: &Connection,
    run_id: AgentRunId,
) -> Result<Option<StoredAgentRunContext>, String> {
    connection
        .query_row(
            "SELECT run_id, host_id, workspace_id, pane_id, provider, active,
                    session_id, session_name, zellij_version, tab_id, tab_name,
                    zellij_pane_id
             FROM agent_run_context WHERE run_id = ?1",
            [run_id.to_string()],
            row_to_record,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(decode_record)
        .transpose()
}

pub(super) type EncodedRecord = (
    String,
    String,
    String,
    Option<String>,
    String,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

pub(super) fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<EncodedRecord> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

pub(super) fn decode_record(encoded: EncodedRecord) -> Result<StoredAgentRunContext, String> {
    let (
        run_id,
        host_id,
        workspace_id,
        pane_id,
        provider,
        active,
        session_id,
        session_name,
        zellij_version,
        tab_id,
        tab_name,
        zellij_pane_id,
    ) = encoded;
    let binding_fields = [
        session_id.is_some(),
        session_name.is_some(),
        zellij_version.is_some(),
        tab_id.is_some(),
        tab_name.is_some(),
        zellij_pane_id.is_some(),
    ];
    if binding_fields.iter().any(|present| *present)
        && !binding_fields.iter().all(|present| *present)
    {
        return Err("Stored agent run has a partial Zellij binding.".to_owned());
    }
    let binding = match (
        session_id,
        session_name,
        zellij_version,
        tab_id,
        tab_name,
        zellij_pane_id,
    ) {
        (
            Some(session_id),
            Some(session_name),
            Some(zellij_version),
            Some(tab_id),
            Some(tab_name),
            Some(zellij_pane_id),
        ) => {
            if tab_id < 0 {
                return Err("Stored agent tab ID is invalid.".to_owned());
            }
            Some(AgentRunBinding {
                session_id: session_id
                    .parse()
                    .map_err(|_| "Stored session ID is invalid.")?,
                session_name,
                zellij_version,
                tab_id: tab_id as u64,
                tab_name,
                zellij_pane_id,
            })
        }
        (None, None, None, None, None, None) => None,
        _ => unreachable!("partial bindings were rejected above"),
    };
    Ok(StoredAgentRunContext {
        context: AgentRunContext {
            host_id: host_id.parse().map_err(|_| "Stored host ID is invalid.")?,
            workspace_id: workspace_id
                .parse()
                .map_err(|_| "Stored workspace ID is invalid.")?,
            run_id: run_id.parse().map_err(|_| "Stored run ID is invalid.")?,
            pane_id: pane_id
                .map(|id| id.parse().map_err(|_| "Stored pane ID is invalid."))
                .transpose()?,
            provider: provider
                .parse()
                .map_err(|_| "Stored provider is invalid.")?,
        },
        binding,
        active,
    })
}

pub(super) fn validate_binding(
    registry: &HostRegistry,
    context: AgentRunContext,
    binding: &AgentRunBinding,
) -> Result<(), String> {
    if context.pane_id.is_none() {
        return Err("Agent run requires a stable pane ID before binding.".to_owned());
    }
    if binding.tab_id > i64::MAX as u64 {
        return Err("Zellij tab ID is outside the supported range.".to_owned());
    }
    for (kind, value) in [
        ("session name", binding.session_name.as_str()),
        ("Zellij version", binding.zellij_version.as_str()),
        ("tab name", binding.tab_name.as_str()),
        ("Zellij pane selector", binding.zellij_pane_id.as_str()),
    ] {
        if value.is_empty()
            || value.len() > 128
            || value.chars().any(|character| character.is_control())
        {
            return Err(format!("Agent {kind} must be a bounded single-line value."));
        }
    }
    let pane_number = binding
        .zellij_pane_id
        .strip_prefix("terminal_")
        .ok_or_else(|| "Agent binding must identify a terminal pane.".to_owned())?;
    if pane_number.parse::<u32>().is_err() {
        return Err("Agent binding has an invalid terminal pane selector.".to_owned());
    }
    if binding.tab_name != format!("agent-{}", context.run_id) {
        return Err("Agent tab name does not match its run ID.".to_owned());
    }
    let session = registry
        .session(binding.session_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Agent binding session is not registered.".to_owned())?;
    if session.workspace_id != context.workspace_id
        || session.backend != SessionBackend::Zellij
        || session.backend_session_id != binding.session_name
        || session.backend_version != binding.zellij_version
        || matches!(session.state, SessionState::Exited | SessionState::Failed)
    {
        return Err("Agent binding does not match the active workspace session.".to_owned());
    }
    Ok(())
}

fn column_exists(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare("PRAGMA table_info(agent_run_context)")?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for column in columns {
        if column? == name {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
