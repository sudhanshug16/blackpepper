use super::super::{session_lease::SessionInitializationLease, ClientRuntime};
use crate::core::{SessionBackend, SessionRecord, SessionState, WorkspaceId, WorkspaceRecord};
use crate::transport::{is_blackpepper_zellij_version, sha256_bytes, PtyProcess};
use crate::zellij::ZellijRuntime;
use portable_pty::PtySize;
use std::path::Path;

impl ClientRuntime {
    pub(crate) fn attach_workspace(
        &mut self,
        workspace_id: WorkspaceId,
        rows: u16,
        cols: u16,
    ) -> Result<(PtyProcess, usize), String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        let result = (|| {
            let size = PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            };
            let (zellij, session, _) = self.ensure_workspace_session_under_lease(&workspace)?;
            let initial = zellij.attach(
                self.transport_mut(workspace.host_id)?,
                &session.backend_session_id,
                Path::new(&workspace.root_path),
                size,
            );
            let (process, clients_before) = match initial {
                Ok(attachment) => attachment,
                Err(crate::zellij::ZellijError::SessionMissingBeforeAttach) => {
                    // The server can exit after discovery but before the
                    // pre-PTY client query. Nothing was attached yet, so
                    // reconcile the durable session exactly once while this
                    // same lifecycle lease still excludes other clients.
                    let (zellij, session, _) =
                        self.ensure_workspace_session_under_lease(&workspace)?;
                    zellij
                        .attach(
                            self.transport_mut(workspace.host_id)?,
                            &session.backend_session_id,
                            Path::new(&workspace.root_path),
                            size,
                        )
                        .map_err(|error| error.to_string())?
                }
                Err(error) => return Err(error.to_string()),
            };
            Ok((
                process,
                super::provisional_attachment_count(clients_before.len()),
            ))
        })();
        // Do not query Zellij again until the embedded terminal reader is
        // running. During attach Zellij can wait for a terminal-size reply and
        // serialize a concurrent `list-clients` behind that handshake. The old
        // synchronous poll therefore deadlocked the whole TUI. This count is a
        // provisional lower bound; the event loop's periodic observation is
        // authoritative once the PTY is being serviced.
        let release = lease.release();
        match (result, release) {
            (Ok(attachment), Ok(())) => Ok(attachment),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(operation), Err(release)) => Err(format!(
                "{operation}; the workspace lifecycle lease also failed to release: {release}"
            )),
        }
    }

    /// Correct Zellij 0.44.3's zero-client background-tab focus only after the
    /// attached PTY reader has started. The lifecycle lease serializes this
    /// revalidation with every Blackpepper attach and background mutation;
    /// the Zellij layer refuses the action unless one exact client remains.
    pub(crate) fn focus_initial_shell_after_first_attach(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<(), String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        let result = (|| {
            let session = self
                .registry
                .sessions_for_workspace(workspace.id)
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|session| {
                    session.backend == SessionBackend::Zellij
                        && session.state != SessionState::Exited
                })
                .max_by_key(|session| session.created_at_ms)
                .ok_or_else(|| {
                    "The attached workspace has no live Zellij session to focus.".to_owned()
                })?;
            let binary =
                self.exact_binary(workspace.host_id, "zellij", &session.backend_version)?;
            let zellij = ZellijRuntime::for_version(binary, &session.backend_version)
                .map_err(|error| error.to_string())?;
            let (zellij, _) = zellij
                .resolve_session_namespace(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                )
                .map_err(|error| error.to_string())?;
            zellij
                .focus_initial_shell_for_single_client(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                )
                .map_err(|error| error.to_string())
        })();
        let release = lease.release();
        match (result, release) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(operation), Err(release)) => Err(format!(
                "{operation}; the workspace lifecycle lease also failed to release: {release}"
            )),
        }
    }

    pub(in crate::client::runtime) fn ensure_workspace_session(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(ZellijRuntime, SessionRecord, bool), String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace.id)?;
        let result = self.ensure_workspace_session_under_lease(&workspace)?;
        crate::transport::CommandCancellation::mask_current(|| lease.release())?;
        Ok(result)
    }

    /// Acquire the host-owned lifecycle gate, then refresh the authoritative
    /// workspace record while still holding it. Every Zellij create, attach,
    /// background-tab mutation, and destroy path uses this same gate.
    pub(in crate::client::runtime) fn acquire_workspace_session_lease(
        &mut self,
        workspace_id: WorkspaceId,
    ) -> Result<(SessionInitializationLease, WorkspaceRecord), String> {
        let workspace = self
            .registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "The selected workspace no longer exists.".to_owned())?;
        let leased_host_id = workspace.host_id;
        // The transient helper acknowledges only after it owns the host-local
        // advisory lock. Acquire it before reading session state so two laptops
        // cannot both act on the same stale registry snapshot.
        let lease = SessionInitializationLease::acquire(self, workspace.host_id, workspace.id)?;
        if workspace.host_id != self.local_host_id {
            super::super::connection::refresh_registry(self, workspace.host_id)?;
        }
        let workspace = self
            .registry
            .workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "The workspace was removed while its session lease was pending.".to_owned()
            })?;
        if workspace.host_id != leased_host_id {
            return Err(
                "The workspace changed hosts while its lifecycle lease was pending; the operation was refused."
                    .to_owned(),
            );
        }
        Ok((lease, workspace))
    }

    pub(in crate::client::runtime) fn ensure_workspace_session_under_lease(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(ZellijRuntime, SessionRecord, bool), String> {
        // A running session owns the Zellij version that created it. Reusing
        // the current release pin here would silently fork the workspace onto
        // a second session after a Blackpepper upgrade.
        let mut session = self.current_or_new_session(workspace)?;
        let zellij_binary =
            self.exact_binary(workspace.host_id, "zellij", &session.backend_version)?;
        let zellij = ZellijRuntime::for_version(zellij_binary, &session.backend_version)
            .map_err(|error| error.to_string())?;
        let config = self.workspace_config(workspace)?;
        let host_configuration_path = {
            let transport = self.transport_mut(workspace.host_id)?;
            zellij
                .check_version(transport)
                .map_err(|error| error.to_string())?;
            zellij
                .user_configuration(transport)
                .map_err(|error| error.to_string())?
                .0
        };
        // The host's own settings are merged in rather than deferred to, so a
        // config that only sets keybindings still gets Blackpepper's
        // appearance and keeps every binding it declared.
        let host_configuration = match host_configuration_path.as_deref() {
            Some(path) => self.read_host_zellij_config(workspace.host_id, path)?,
            None => None,
        };
        let zellij = {
            let path = self.managed_zellij_config_path(
                workspace.host_id,
                &session.backend_version,
                host_configuration.as_deref(),
            )?;
            let zellij = zellij
                .with_config_file(path)
                .map_err(|error| error.to_string())?;
            zellij
                .check_configuration(self.transport_mut(workspace.host_id)?)
                .map_err(|error| error.to_string())?;
            zellij
        };
        let (zellij, session_exists) = {
            let transport = self.transport_mut(workspace.host_id)?;
            zellij
                .resolve_session_namespace(transport, &session.backend_session_id)
                .map_err(|error| error.to_string())?
        };
        if !session_exists {
            // The persisted name can outlive the Zellij server across a host
            // reboot. Retire that process generation before startup services
            // are allowed to reuse its numeric pane IDs.
            self.end_runs_for_recreated_session(
                workspace.host_id,
                workspace.id,
                &session.backend_session_id,
            )?;
            // `Starting` is a durable recovery barrier: if creation or service
            // startup loses its response, the next client finishes the same
            // idempotent initialization instead of assuming it completed.
            session.state = SessionState::Starting;
            session.touch();
            self.persist_session(workspace.host_id, &session)?;
        }
        let initialization_pending = session.state == SessionState::Starting;
        let created = if session_exists {
            false
        } else {
            zellij
                .ensure_session_with_env(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                    Path::new(&workspace.root_path),
                    &config.workspace_env,
                )
                .map_err(|error| error.to_string())?
        };
        if initialization_pending && matches!(workspace.setup, crate::core::WorkspaceSetup::Ready) {
            self.start_configured_services(&zellij, &session, workspace)?;
        }
        session.state = SessionState::Running;
        session.touch();
        self.persist_session(workspace.host_id, &session)?;
        Ok((zellij, session, created))
    }

    pub(crate) fn terminate_workspace(&mut self, workspace_id: WorkspaceId) -> Result<(), String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        self.terminate_workspace_under_lease(&workspace)?;
        lease.release()
    }

    pub(in crate::client::runtime) fn terminate_workspace_under_lease(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(), String> {
        let Some(mut session) = self
            .registry
            .sessions_for_workspace(workspace.id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|session| session.backend == SessionBackend::Zellij)
            .max_by_key(|session| session.created_at_ms)
        else {
            return Ok(());
        };
        let binary = self.exact_binary(workspace.host_id, "zellij", &session.backend_version)?;
        let zellij = ZellijRuntime::for_version(binary, &session.backend_version)
            .map_err(|error| error.to_string())?;
        let (zellij, session_exists) = zellij
            .resolve_session_namespace(
                self.transport_mut(workspace.host_id)?,
                &session.backend_session_id,
            )
            .map_err(|error| error.to_string())?;
        if !session_exists {
            session.state = SessionState::Exited;
            session.touch();
            return self.persist_session(workspace.host_id, &session);
        }
        let mut clients = Vec::new();
        for attempt in 0..=20 {
            clients = zellij
                .list_clients(
                    self.transport_mut(workspace.host_id)?,
                    &session.backend_session_id,
                )
                .map_err(|error| error.to_string())?;
            if clients.is_empty() {
                break;
            }
            if attempt < 20 {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        if !clients.is_empty() {
            return Err(format!(
                "The Zellij session still has {} attached client(s); detach them before terminating the workspace.",
                clients.len()
            ));
        }
        zellij
            .kill_session(
                self.transport_mut(workspace.host_id)?,
                &session.backend_session_id,
            )
            .map_err(|error| error.to_string())?;
        session.state = SessionState::Exited;
        session.touch();
        self.persist_session(workspace.host_id, &session)
    }

    pub(crate) fn mark_detached(&mut self, workspace_id: WorkspaceId) -> Result<(), String> {
        let (lease, workspace) = self.acquire_workspace_session_lease(workspace_id)?;
        if let Some(mut session) = self
            .registry
            .sessions_for_workspace(workspace_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .max_by_key(|session| session.created_at_ms)
            .filter(|session| session.state != SessionState::Exited)
        {
            session.state = SessionState::Detached;
            session.touch();
            self.persist_session(workspace.host_id, &session)?;
        }
        lease.release()
    }

    pub(in crate::client::runtime) fn current_or_new_session(
        &self,
        workspace: &WorkspaceRecord,
    ) -> Result<SessionRecord, String> {
        if let Some(session) = self
            .registry
            .sessions_for_workspace(workspace.id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|session| {
                session.backend == SessionBackend::Zellij && session.state != SessionState::Exited
            })
            .max_by_key(|session| session.created_at_ms)
        {
            return Ok(session);
        }
        Ok(SessionRecord::new(
            workspace.id,
            SessionBackend::Zellij,
            crate::transport::ZELLIJ_VERSION,
            zellij_session_name(workspace.id, crate::transport::ZELLIJ_VERSION),
        ))
    }
}

const BRANDED_SESSION_HASH_LENGTH: usize = 12;

/// Keep stock sessions discoverable by released Blackpepper clients while
/// preventing a branded client from attaching to a stock or older branded
/// server that implements a different terminal-forwarding contract.
pub(super) fn zellij_session_name(workspace_id: WorkspaceId, version: &str) -> String {
    let legacy_name = format!("bp-{workspace_id}");
    if !is_blackpepper_zellij_version(version) {
        return legacy_name;
    }
    let version_hash = sha256_bytes(version.as_bytes());
    format!(
        "{legacy_name}-{}",
        &version_hash[..BRANDED_SESSION_HASH_LENGTH]
    )
}
