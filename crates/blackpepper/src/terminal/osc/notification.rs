//! Validation and canonical forwarding for terminal notification escapes.

pub(super) const MAX_NOTIFICATION_FIELD_CHARS: usize = 1024;

/// Rebuild a complete notification escape for the real outer terminal.
/// Blackpepper does not interpret the sender or notification contents.
pub(super) fn notification_sequence(command: &[u8]) -> Option<Vec<u8>> {
    if let Some(payload) = command.strip_prefix(b"9;") {
        let text = sanitize_field(std::str::from_utf8(payload).ok()?);
        return (!text.is_empty()).then(|| format!("\x1b]9;{text}\x07").into_bytes());
    }
    if let Some(payload) = command.strip_prefix(b"777;notify;") {
        let payload = std::str::from_utf8(payload).ok()?;
        let (title, body) = payload.split_once(';').unwrap_or(("", payload));
        let title = sanitize_field(title);
        let body = sanitize_field(body);
        return (!title.is_empty() || !body.is_empty())
            .then(|| format!("\x1b]777;notify;{title};{body}\x07").into_bytes());
    }
    let payload = command.strip_prefix(b"99;")?;
    (payload.contains(&b';') && safe_osc_99(payload)).then(|| {
        let mut sequence = Vec::with_capacity(command.len() + 3);
        sequence.extend_from_slice(b"\x1b]");
        sequence.extend_from_slice(command);
        sequence.push(0x07);
        sequence
    })
}

fn sanitize_field(field: &str) -> String {
    field
        .chars()
        .filter(|character| {
            *character == '\t'
                || (!character.is_control() && !matches!(u32::from(*character), 0x80..=0x9f))
        })
        .take(MAX_NOTIFICATION_FIELD_CHARS)
        .collect()
}

fn safe_osc_99(payload: &[u8]) -> bool {
    payload.len() <= MAX_NOTIFICATION_FIELD_CHARS * 4
        && std::str::from_utf8(payload)
            .is_ok_and(|text| text.chars().all(|character| !character.is_control()))
}
