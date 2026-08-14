use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use crate::transport::{CommandOutput, HostCommand, HostTransport, TransportError};

use super::super::model::{checked, ZellijError};

const METADATA_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const METADATA_RETRY_POLL: Duration = Duration::from_millis(25);
const METADATA_MAX_ATTEMPTS: usize = 3;

/// Read one JSON snapshot without treating Zellij's blank-success race as an
/// empty authoritative result. Every retry is read-only and the whole attempt
/// stays inside the caller's original metadata deadline.
pub(super) fn read_json<T: DeserializeOwned>(
    host: &mut dyn HostTransport,
    command: &HostCommand,
    operation: &str,
    zellij_timeout_message: &str,
    json_kind: &str,
    timeout: Duration,
) -> Result<T, ZellijError> {
    let output = read_output(host, command, timeout, |output| {
        transient_metadata_result(output, zellij_timeout_message)
    })?;
    if transient_metadata_result(&output, zellij_timeout_message) {
        let transient = if output.success {
            "Zellij returned success with blank metadata"
        } else {
            "Zellij reported its internal metadata timeout"
        };
        return Err(metadata_attempts_error(operation, timeout, transient));
    }
    let output = checked(output, operation)?;
    serde_json::from_slice(&output.stdout)
        .map_err(|error| ZellijError::InvalidOutput(format!("invalid {json_kind} JSON: {error}")))
}

/// Retry one idempotent metadata command without changing the meaning of its
/// final output. Callers that assign semantics to an exact missing-session
/// response can therefore retry the race and still classify a real absence
/// after the bounded final attempt.
pub(super) fn read_output(
    host: &mut dyn HostTransport,
    command: &HostCommand,
    timeout: Duration,
    is_transient: impl Fn(&CommandOutput) -> bool,
) -> Result<CommandOutput, ZellijError> {
    let started = Instant::now();
    for attempt in 0..METADATA_MAX_ATTEMPTS {
        let remaining = timeout.saturating_sub(started.elapsed());
        let probe_timeout = METADATA_PROBE_TIMEOUT.min(remaining);
        let output = match host.exec_timeout(command, probe_timeout) {
            Ok(output) => output,
            Err(
                error @ TransportError::CommandTimedOut {
                    cancellation_error: None,
                    ..
                },
            ) => {
                if attempt + 1 >= METADATA_MAX_ATTEMPTS || started.elapsed() >= timeout {
                    return Err(error.into());
                }
                sleep_until_next_probe(started, timeout);
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if !is_transient(&output)
            || attempt + 1 >= METADATA_MAX_ATTEMPTS
            || started.elapsed() >= timeout
        {
            return Ok(output);
        }
        sleep_until_next_probe(started, timeout);
    }
    unreachable!("the bounded metadata loop always returns on its final attempt")
}

/// Zellij 0.44.3 can unblock a metadata CLI with no payload, or report its
/// own one-second screen-query timeout. Match only those exact results so
/// malformed non-empty output and unrelated command failures still fail
/// closed as protocol drift.
pub(super) fn transient_metadata_result(
    output: &CommandOutput,
    zellij_timeout_message: &str,
) -> bool {
    if output.success
        && output.stderr.is_empty()
        && output.stdout.iter().all(u8::is_ascii_whitespace)
    {
        return true;
    }
    !output.success
        && output.status == Some(2)
        && output.stdout.is_empty()
        && std::str::from_utf8(&output.stderr).ok().map(str::trim) == Some(zellij_timeout_message)
}

fn sleep_until_next_probe(started: Instant, timeout: Duration) {
    std::thread::sleep(METADATA_RETRY_POLL.min(timeout.saturating_sub(started.elapsed())));
}

fn metadata_attempts_error(
    operation: &str,
    timeout: Duration,
    last_transient: &str,
) -> ZellijError {
    ZellijError::InvalidOutput(format!(
        "{operation} returned no complete JSON after bounded retries within {}ms; {last_transient}",
        timeout.as_millis()
    ))
}
