use super::wire::{read_bounded_line, write_response, LineRead};
use super::{
    FailureCode, HelperRequest, HelperResponse, ProtocolFailure, RequestOperation, ResponsePayload,
    ResponseResult, PROTOCOL_VERSION,
};
use crate::core::{HostRegistry, ProtocolError};
use std::io::{BufRead, Write};

/// Extension point used by the transient host helper. Registry-only callers
/// can continue to use `serve_json_lines` without starting host processes.
pub trait ProtocolExtension {
    fn execute(&mut self, registry: &HostRegistry, operation: RequestOperation) -> ResponseResult;
}

struct NoProtocolExtension;

impl ProtocolExtension for NoProtocolExtension {
    fn execute(
        &mut self,
        _registry: &HostRegistry,
        _operation: RequestOperation,
    ) -> ResponseResult {
        ResponseResult::Error {
            error: ProtocolFailure {
                code: FailureCode::UnsupportedOperation,
                message: "this protocol server does not provide host services".to_owned(),
            },
        }
    }
}

/// Serves the versioned protocol until EOF. Each request and response occupies one JSON line.
pub fn serve_json_lines<R: BufRead, W: Write>(
    registry: &HostRegistry,
    reader: R,
    writer: W,
) -> Result<(), ProtocolError> {
    serve_json_lines_with_extension(registry, &mut NoProtocolExtension, reader, writer)
}

pub fn serve_json_lines_with_extension<R: BufRead, W: Write>(
    registry: &HostRegistry,
    extension: &mut impl ProtocolExtension,
    mut reader: R,
    mut writer: W,
) -> Result<(), ProtocolError> {
    let mut handshaken = false;
    loop {
        let line = match read_bounded_line(&mut reader)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong => {
                write_response(
                    &mut writer,
                    failure(None, FailureCode::InvalidRequest, "request exceeds 1 MiB"),
                )?;
                continue;
            }
            LineRead::Line(line) => line,
        };
        let request: HelperRequest = match serde_json::from_slice(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut writer,
                    failure(
                        None,
                        FailureCode::InvalidRequest,
                        format!("invalid request JSON: {error}"),
                    ),
                )?;
                continue;
            }
        };
        if request.protocol_version != PROTOCOL_VERSION {
            write_response(
                &mut writer,
                failure(
                    Some(request.request_id),
                    FailureCode::VersionMismatch,
                    format!(
                        "protocol {} is unsupported; expected {}",
                        request.protocol_version, PROTOCOL_VERSION
                    ),
                ),
            )?;
            continue;
        }
        if !handshaken && !matches!(request.operation, RequestOperation::Handshake { .. }) {
            write_response(
                &mut writer,
                failure(
                    Some(request.request_id),
                    FailureCode::HandshakeRequired,
                    "handshake must be the first valid request",
                ),
            )?;
            continue;
        }
        let result = match request.operation {
            RequestOperation::Handshake { client_version } if client_version != crate::BUILD_ID => {
                ResponseResult::Error {
                    error: ProtocolFailure {
                        code: FailureCode::VersionMismatch,
                        message: format!(
                            "client build {client_version:?} does not match helper build {:?}",
                            crate::BUILD_ID,
                        ),
                    },
                }
            }
            RequestOperation::Handshake { .. } => match registry.local_host_id() {
                Ok(host_id) => {
                    handshaken = true;
                    ResponseResult::Ok {
                        payload: ResponsePayload::Handshake {
                            helper_version: crate::BUILD_ID.to_owned(),
                            protocol_version: PROTOCOL_VERSION,
                            host_id,
                        },
                    }
                }
                Err(error) => ResponseResult::Error {
                    error: ProtocolFailure {
                        code: FailureCode::RegistryError,
                        message: error.to_string(),
                    },
                },
            },
            operation => execute_operation(registry, extension, operation),
        };
        write_response(
            &mut writer,
            HelperResponse {
                request_id: Some(request.request_id),
                protocol_version: PROTOCOL_VERSION,
                result,
            },
        )?;
    }
}

fn execute_operation(
    registry: &HostRegistry,
    extension: &mut impl ProtocolExtension,
    operation: RequestOperation,
) -> ResponseResult {
    let result = match operation {
        RequestOperation::Snapshot => registry
            .snapshot()
            .map(|snapshot| ResponsePayload::Snapshot { snapshot }),
        RequestOperation::UpsertWorkspace { workspace } => registry
            .upsert_workspace(&workspace)
            .map(|()| ResponsePayload::Acknowledged),
        RequestOperation::UpsertSession { session } => registry
            .upsert_session(&session)
            .map(|()| ResponsePayload::Acknowledged),
        RequestOperation::RemoveWorkspace { workspace_id } => registry
            .remove_workspace(workspace_id)
            .map(|existed| ResponsePayload::Removed { existed }),
        RequestOperation::RemoveSession { session_id } => registry
            .remove_session(session_id)
            .map(|existed| ResponsePayload::Removed { existed }),
        RequestOperation::Handshake { .. } => unreachable!("handshake handled by caller"),
        operation => return extension.execute(registry, operation),
    };
    match result {
        Ok(payload) => ResponseResult::Ok { payload },
        Err(error) => ResponseResult::Error {
            error: ProtocolFailure {
                code: FailureCode::RegistryError,
                message: error.to_string(),
            },
        },
    }
}

fn failure(
    request_id: Option<u64>,
    code: FailureCode,
    message: impl Into<String>,
) -> HelperResponse {
    HelperResponse {
        request_id,
        protocol_version: PROTOCOL_VERSION,
        result: ResponseResult::Error {
            error: ProtocolFailure {
                code,
                message: message.into(),
            },
        },
    }
}
