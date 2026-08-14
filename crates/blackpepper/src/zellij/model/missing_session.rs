use crate::transport::CommandOutput;

/// Zellij 0.44.3 reports this exact result for an absent session. Its
/// short-lived CLI client race can also report the same result transiently for
/// a live session, so callers must retry before assigning absence semantics.
/// Keep the classification narrow so arbitrary command failures never become
/// a retry or attach race.
pub(crate) fn client_list_reports_missing_session(output: &CommandOutput, session: &str) -> bool {
    if reports_no_active_session(output) {
        return true;
    }

    // With another detached session, 0.44.3 exits 0 and writes ANSI-formatted
    // session rows to stdout. With an attached session, it exits 1 and appends
    // plain rows to stderr. The rows are presentation, not evidence we use;
    // bind the classification to the exact requested-name header, status, and
    // stream placement instead of trying to parse Zellij's colored display.
    let Ok(stderr) = std::str::from_utf8(&output.stderr) else {
        return false;
    };
    let expected = format!("Session '{session}' not found. The following sessions are active:");
    match output.status {
        Some(0)
            if output.success
                && stderr == format!("{expected}\n")
                && valid_reported_session_rows(&output.stdout) =>
        {
            true
        }
        Some(1) if !output.success && output.stdout.is_empty() => stderr
            .strip_prefix(&format!("{expected}\n"))
            .is_some_and(|rows| valid_reported_session_rows(rows.as_bytes())),
        _ => false,
    }
}

/// The exact false-absence result emitted by Zellij's recycled CLI client
/// race. This predicate deliberately excludes named missing-session reports.
pub(crate) fn reports_no_active_session(output: &CommandOutput) -> bool {
    let Ok(stderr) = std::str::from_utf8(&output.stderr) else {
        return false;
    };
    !output.success
        && output.status == Some(1)
        && output.stdout.is_empty()
        && stderr.trim_end_matches(['\n', '\r']) == "There is no active session!"
}

fn valid_reported_session_rows(value: &[u8]) -> bool {
    if value.is_empty() || value.len() > 64 * 1024 || !value.ends_with(b"\n") || value.contains(&0)
    {
        return false;
    }
    let mut rows = value
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty());
    rows.next().is_some_and(|line| line != b"\r") && rows.all(|line| line != b"\r")
}
