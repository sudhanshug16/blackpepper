//! Allowlisted services exposed by the transient `bp-host` process.

mod agent_context;
mod agent_events;
mod blocker_watch;
mod periodic;
mod ports;
mod process;
mod provider_hook;
mod repository;
mod session_lease;
mod tool_runtime;
mod worktrunk_approval;
mod worktrunk_exec;
mod worktrunk_lock;

pub use blocker_watch::{watch_blockers, watch_blockers_cancellable, BlockerWatchArgs};
pub use provider_hook::{record_provider_hook, ProviderHookArgs};
pub use session_lease::{hold_session_lease, SessionLeaseArgs, SESSION_LEASE_READY};

use crate::core::{
    CorePaths, FailureCode, HostRegistry, HostServicePayload, ProtocolExtension, ProtocolFailure,
    RequestOperation, ResponsePayload, ResponseResult,
};
use agent_events::{AgentRunContext, HostAgentEvents};
use worktrunk_exec::WorktrunkExecutor;

pub struct HostServices {
    paths: CorePaths,
    worktrunk: WorktrunkExecutor,
}

impl HostServices {
    pub fn new(paths: CorePaths) -> Self {
        let worktrunk = WorktrunkExecutor::discover(&paths);
        Self { paths, worktrunk }
    }

    #[cfg(test)]
    fn with_worktrunk(paths: CorePaths, binary: std::path::PathBuf) -> Self {
        let worktrunk = WorktrunkExecutor::with_binary(&paths, binary);
        Self { paths, worktrunk }
    }

    fn agent_events(&self) -> Result<HostAgentEvents, String> {
        HostAgentEvents::open(&self.paths)
    }
}

impl ProtocolExtension for HostServices {
    fn execute(&mut self, registry: &HostRegistry, operation: RequestOperation) -> ResponseResult {
        let result = match operation {
            RequestOperation::DiscoverPorts => Ok(HostServicePayload::Ports {
                snapshot: ports::discover(registry),
            }),
            RequestOperation::PeriodicRefresh {
                attached_workspaces,
            } => periodic::refresh(&self.paths, registry, attached_workspaces).map(|refresh| {
                HostServicePayload::PeriodicRefresh {
                    refresh: Box::new(refresh),
                }
            }),
            RequestOperation::InspectRepository { root_path } => {
                repository::inspect(registry, &root_path)
                    .map(|repository| HostServicePayload::RepositoryInspected { repository })
            }
            RequestOperation::RegisterWorkspace {
                root_path,
                display_name,
            } => repository::register(registry, &root_path, display_name)
                .map(|workspace| HostServicePayload::WorkspaceRegistered { workspace }),
            RequestOperation::AgentSnapshot { run_id } => self
                .agent_events()
                .and_then(|mut events| events.snapshot(run_id))
                .map(|snapshot| HostServicePayload::AgentSnapshot { snapshot }),
            RequestOperation::AgentFollow {
                run_id,
                after_sequence,
                limit,
            } => self
                .agent_events()
                .and_then(|events| events.follow(run_id, after_sequence, limit))
                .map(|updates| HostServicePayload::AgentUpdates { updates }),
            RequestOperation::RegisterAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
            } => {
                let result = registry
                    .local_host_id()
                    .map_err(|error| error.to_string())
                    .and_then(|host_id| {
                        let mut events = self.agent_events()?;
                        events.register_run(
                            registry,
                            AgentRunContext {
                                host_id,
                                workspace_id,
                                run_id,
                                pane_id,
                                provider,
                            },
                        )
                    });
                return match result {
                    Ok(()) => ResponseResult::Ok {
                        payload: ResponsePayload::Acknowledged,
                    },
                    Err(message) => service_error(message),
                };
            }
            RequestOperation::BindAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
                binding,
            } => {
                let result = local_agent_context(registry, workspace_id, run_id, pane_id, provider)
                    .and_then(|context| self.agent_events()?.bind_run(registry, context, &binding));
                return acknowledged(result);
            }
            RequestOperation::AbortAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
            } => {
                let result = local_agent_context(registry, workspace_id, run_id, pane_id, provider)
                    .and_then(|context| self.agent_events()?.abort_run(context));
                return acknowledged(result);
            }
            RequestOperation::ListAgentRuns { workspace_id } => workspace_id
                .map_or(Ok(()), |id| validate_agent_workspace(registry, id))
                .and_then(|()| self.agent_events())
                .and_then(|mut events| events.list_runs(workspace_id))
                .map(|runs| HostServicePayload::AgentRuns { runs }),
            RequestOperation::ReconcileAgentRun {
                workspace_id,
                run_id,
                pane_id,
                provider,
                binding,
                observation,
            } => local_agent_context(registry, workspace_id, run_id, pane_id, provider)
                .and_then(|context| {
                    self.agent_events()?
                        .reconcile_run(context, &binding, observation)
                })
                .map(|run| HostServicePayload::AgentRunReconciled { run: Box::new(run) }),
            RequestOperation::WorktrunkList {
                workspace_id,
                repository_path,
            } => self
                .worktrunk
                .list(registry, workspace_id, &repository_path),
            RequestOperation::WorktrunkCreate {
                repository_path,
                branch,
                base,
                approval,
            } => self.worktrunk.create(
                &repository_path,
                &branch,
                base.as_deref(),
                approval.as_ref(),
            ),
            RequestOperation::WorktrunkSwitch {
                repository_path,
                selector,
                approval,
            } => self
                .worktrunk
                .switch(&repository_path, &selector, approval.as_ref()),
            RequestOperation::WorktrunkRemove {
                workspace_id,
                target_path,
                approval,
            } => self
                .worktrunk
                .remove(registry, workspace_id, &target_path, approval.as_ref()),
            _ => {
                return ResponseResult::Error {
                    error: ProtocolFailure {
                        code: FailureCode::UnsupportedOperation,
                        message: "operation is not a host service".to_owned(),
                    },
                };
            }
        };
        match result {
            Ok(payload) => ResponseResult::Ok {
                payload: ResponsePayload::HostService {
                    payload: Box::new(payload),
                },
            },
            Err(message) => service_error(message),
        }
    }
}

fn service_error(message: String) -> ResponseResult {
    ResponseResult::Error {
        error: ProtocolFailure {
            code: FailureCode::HostServiceError,
            message,
        },
    }
}

fn acknowledged(result: Result<(), String>) -> ResponseResult {
    match result {
        Ok(()) => ResponseResult::Ok {
            payload: ResponsePayload::Acknowledged,
        },
        Err(message) => service_error(message),
    }
}

fn local_agent_context(
    registry: &HostRegistry,
    workspace_id: crate::core::WorkspaceId,
    run_id: crate::core::AgentRunId,
    pane_id: crate::core::PaneId,
    provider: crate::agent_status::Provider,
) -> Result<AgentRunContext, String> {
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    Ok(AgentRunContext {
        host_id,
        workspace_id,
        run_id,
        pane_id: Some(pane_id),
        provider,
    })
}

fn validate_agent_workspace(
    registry: &HostRegistry,
    workspace_id: crate::core::WorkspaceId,
) -> Result<(), String> {
    let host_id = registry
        .local_host_id()
        .map_err(|error| error.to_string())?;
    let workspace = registry
        .workspace(workspace_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "Agent run workspace is not registered.".to_owned())?;
    if workspace.host_id != host_id {
        return Err("Agent run workspace belongs to another host.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
