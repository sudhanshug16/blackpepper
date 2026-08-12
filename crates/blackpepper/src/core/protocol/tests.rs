use super::*;
use crate::core::HostRegistry;
use std::io::Cursor;

#[test]
fn handshake_gates_registry_operations_and_returns_stable_host_id() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    let host_id = registry.ensure_local_host("test-host").unwrap();
    let requests = vec![
        request(1, RequestOperation::Snapshot),
        request(
            2,
            RequestOperation::Handshake {
                client_version: crate::BUILD_ID.to_owned(),
            },
        ),
        request(3, RequestOperation::Snapshot),
    ];
    let input = requests
        .into_iter()
        .map(|request| serde_json::to_string(&request).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = Vec::new();
    serve_json_lines(&registry, Cursor::new(input), &mut output).unwrap();
    let responses: Vec<HelperResponse> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert!(matches!(
        &responses[0].result,
        ResponseResult::Error { error } if error.code == FailureCode::HandshakeRequired
    ));
    assert!(matches!(
        &responses[1].result,
        ResponseResult::Ok {
            payload: ResponsePayload::Handshake {
                helper_version,
                host_id: returned,
                ..
            }
        } if *returned == host_id && helper_version == crate::BUILD_ID
    ));
    assert!(matches!(
        &responses[2].result,
        ResponseResult::Ok {
            payload: ResponsePayload::Snapshot { snapshot }
        } if snapshot.hosts.len() == 1 && snapshot.hosts[0].id == host_id
    ));
}

#[test]
fn rejects_incompatible_protocol_versions() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    registry.ensure_local_host("test-host").unwrap();
    let mut handshake = request(
        7,
        RequestOperation::Handshake {
            client_version: crate::BUILD_ID.to_owned(),
        },
    );
    handshake.protocol_version += 1;
    let mut output = Vec::new();
    serve_json_lines(
        &registry,
        Cursor::new(serde_json::to_string(&handshake).unwrap()),
        &mut output,
    )
    .unwrap();
    let response: HelperResponse = serde_json::from_slice(&output).unwrap();
    assert!(matches!(
        response.result,
        ResponseResult::Error { error } if error.code == FailureCode::VersionMismatch
    ));
}

#[test]
fn rejects_a_different_build_before_registry_operations() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = HostRegistry::open(root.path().join("registry.sqlite3")).unwrap();
    registry.ensure_local_host("test-host").unwrap();
    let requests = [
        request(
            1,
            RequestOperation::Handshake {
                client_version: "same-package-version-but-different-build".to_owned(),
            },
        ),
        request(2, RequestOperation::Snapshot),
    ];
    let input = requests
        .into_iter()
        .map(|request| serde_json::to_string(&request).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = Vec::new();
    serve_json_lines(&registry, Cursor::new(input), &mut output).unwrap();
    let responses = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<HelperResponse>(line).unwrap())
        .collect::<Vec<_>>();

    assert!(matches!(
        &responses[0].result,
        ResponseResult::Error { error } if error.code == FailureCode::VersionMismatch
    ));
    assert!(matches!(
        &responses[1].result,
        ResponseResult::Error { error } if error.code == FailureCode::HandshakeRequired
    ));
}

fn request(request_id: u64, operation: RequestOperation) -> HelperRequest {
    HelperRequest {
        request_id,
        protocol_version: PROTOCOL_VERSION,
        operation,
    }
}
