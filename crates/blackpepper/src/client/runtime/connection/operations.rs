use super::super::{helper, ClientRuntime};
use crate::core::{
    HelperRequest, HostId, RequestOperation, ResponsePayload, ResponseResult, PROTOCOL_VERSION,
};
use crate::transport::{HostCommand, HostTransport, SshTransport};
use std::io::Write;

#[derive(Debug)]
pub(in crate::client::runtime) enum RegistryOperationError {
    BeforeSend(String),
    UnknownAfterSend(String),
    Rejected(String),
}

impl RegistryOperationError {
    pub fn is_unknown_after_send(&self) -> bool {
        matches!(self, Self::UnknownAfterSend(_))
    }
}

impl std::fmt::Display for RegistryOperationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeSend(message)
            | Self::UnknownAfterSend(message)
            | Self::Rejected(message) => formatter.write_str(message),
        }
    }
}

pub(in crate::client::runtime) fn refresh_registry(
    runtime: &mut ClientRuntime,
    host_id: HostId,
) -> Result<(), String> {
    if host_id == runtime.local_host_id {
        return Ok(());
    }
    let payload = registry_operation(runtime, host_id, RequestOperation::Snapshot)?;
    let ResponsePayload::Snapshot { snapshot } = payload else {
        return Err("bp-host returned an unexpected registry refresh response.".to_string());
    };
    super::reconcile_remote_snapshot(runtime, host_id, &snapshot)
}

pub(super) fn path_text(path: &std::path::Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Remote path is not valid UTF-8: {}", path.display()))
}

pub(super) fn helper_exchange(
    transport: &mut SshTransport,
    helper: &str,
) -> Result<Vec<crate::core::HelperResponse>, String> {
    let operations = [
        RequestOperation::Handshake {
            client_version: crate::BUILD_ID.to_owned(),
        },
        RequestOperation::Snapshot,
    ];
    let mut child = transport
        .spawn_exec_with_stdin(&HostCommand::new(helper))
        .map_err(|error| error.to_string())?;
    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| "bp-host helper stdin was unavailable.".to_string())?;
    for (index, operation) in operations.into_iter().enumerate() {
        serde_json::to_writer(
            &mut stdin,
            &HelperRequest {
                request_id: index as u64 + 1,
                protocol_version: PROTOCOL_VERSION,
                operation,
            },
        )
        .map_err(|error| error.to_string())?;
        stdin.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.success {
        return Err(format!(
            "bp-host failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).map_err(|error| error.to_string()))
        .collect()
}

pub(in crate::client::runtime) fn registry_operation(
    runtime: &mut ClientRuntime,
    host_id: HostId,
    operation: RequestOperation,
) -> Result<ResponsePayload, String> {
    registry_operation_tracked(runtime, host_id, operation).map_err(|error| error.to_string())
}

pub(in crate::client::runtime) fn registry_operation_tracked(
    runtime: &mut ClientRuntime,
    host_id: HostId,
    operation: RequestOperation,
) -> Result<ResponsePayload, RegistryOperationError> {
    let helper = if host_id == runtime.local_host_id {
        local_helper_path().map_err(RegistryOperationError::BeforeSend)?
    } else {
        runtime.helper_paths.get(&host_id).cloned().ok_or_else(|| {
            RegistryOperationError::BeforeSend("The remote helper is not ready.".to_string())
        })?
    };
    let operations = [
        RequestOperation::Handshake {
            client_version: crate::BUILD_ID.to_owned(),
        },
        operation,
    ];
    let mut requests = Vec::new();
    for (index, operation) in operations.into_iter().enumerate() {
        serde_json::to_writer(
            &mut requests,
            &HelperRequest {
                request_id: index as u64 + 1,
                protocol_version: PROTOCOL_VERSION,
                operation,
            },
        )
        .map_err(|error| RegistryOperationError::BeforeSend(error.to_string()))?;
        requests.push(b'\n');
    }
    let transport = runtime
        .transport_mut(host_id)
        .map_err(RegistryOperationError::BeforeSend)?;
    let mut child = transport
        .spawn_exec_with_stdin(&HostCommand::new(helper))
        .map_err(|error| RegistryOperationError::BeforeSend(error.to_string()))?;
    let mut stdin = child.take_stdin().ok_or_else(|| {
        RegistryOperationError::BeforeSend("bp-host helper stdin was unavailable.".to_string())
    })?;
    stdin.write_all(&requests).map_err(|error| {
        RegistryOperationError::UnknownAfterSend(format!(
            "bp-host request channel failed after dispatch began: {error}"
        ))
    })?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| RegistryOperationError::UnknownAfterSend(error.to_string()))?;
    if !output.success {
        return Err(RegistryOperationError::UnknownAfterSend(format!(
            "bp-host failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let response = stdout.lines().last().ok_or_else(|| {
        RegistryOperationError::UnknownAfterSend(
            "bp-host returned no operation response.".to_string(),
        )
    })?;
    let response: crate::core::HelperResponse =
        serde_json::from_str(response).map_err(|error| {
            RegistryOperationError::UnknownAfterSend(format!(
                "bp-host returned an invalid operation response: {error}"
            ))
        })?;
    match response.result {
        ResponseResult::Ok { payload } => Ok(payload),
        ResponseResult::Error { error } => Err(RegistryOperationError::Rejected(error.message)),
    }
}

fn local_helper_path() -> Result<String, String> {
    path_text(&helper::sibling_helper_path()?)
}

pub(super) fn handshake_host_id(
    responses: &[crate::core::HelperResponse],
) -> Result<HostId, String> {
    responses
        .iter()
        .find_map(|response| match &response.result {
            ResponseResult::Ok {
                payload:
                    ResponsePayload::Handshake {
                        helper_version,
                        host_id,
                        ..
                    },
            } => Some((helper_version, *host_id)),
            _ => None,
        })
        .ok_or_else(|| "bp-host handshake did not return its stable host ID.".to_string())
        .and_then(|(helper_version, host_id)| {
            if helper_version == crate::BUILD_ID {
                Ok(host_id)
            } else {
                Err(format!(
                    "bp-host build {helper_version:?} does not match client build {:?}.",
                    crate::BUILD_ID,
                ))
            }
        })
}
