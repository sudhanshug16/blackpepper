use super::*;

#[test]
fn handles_split_clipboard_write_without_returning_evidence() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert!(protocol.process(b"\x1b").is_empty());
    assert!(protocol.process(b"]52;c;aGV").is_empty());
    assert_eq!(
        protocol.process(b"sbG8=\x1b\\"),
        vec![OscAction::SetClipboard {
            target: ClipboardTarget::System,
            text: "hello".to_string(),
        }]
    );
}

#[test]
fn preserves_primary_selection_and_rejects_unknown_targets() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert_eq!(
        protocol.process(b"\x1b]52;p;aGVsbG8=\x07"),
        vec![OscAction::SetClipboard {
            target: ClipboardTarget::Primary,
            text: "hello".to_string(),
        }]
    );
    assert!(protocol.process(b"\x1b]52;s;aGVsbG8=\x07").is_empty());
}

#[test]
fn clipboard_read_is_ignored() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert!(protocol.process(b"\x1b]52;c;?\x07").is_empty());
}

#[test]
fn outer_clipboard_write_is_bounded_and_normalized() {
    assert_eq!(
        clipboard_write_sequence(ClipboardTarget::System, "hello"),
        Some(b"\x1b]52;c;aGVsbG8=\x07".to_vec())
    );
    assert_eq!(
        clipboard_write_sequence(ClipboardTarget::Primary, "hello"),
        Some(b"\x1b]52;p;aGVsbG8=\x07".to_vec())
    );
    assert!(clipboard_write_sequence(
        ClipboardTarget::System,
        &"x".repeat(MAX_CLIPBOARD_BYTES + 1)
    )
    .is_none());
}

#[test]
fn split_clipboard_write_accepts_the_full_decoded_limit() {
    let text = "x".repeat(MAX_CLIPBOARD_BYTES);
    let sequence = clipboard_write_sequence(ClipboardTarget::System, &text).unwrap();
    let split = sequence.len() - 1;
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert!(protocol.process(&sequence[..split]).is_empty());
    let actions = protocol.process(&sequence[split..]);
    assert!(matches!(
        actions.as_slice(),
        [OscAction::SetClipboard {
            target: ClipboardTarget::System,
            text: decoded,
        }] if decoded.len() == MAX_CLIPBOARD_BYTES
    ));
}

#[test]
fn replies_to_terminal_color_queries() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert_eq!(
        protocol.process(b"\x1b]10;?\x07\x1b]11;?\x07"),
        vec![
            OscAction::WriteToPty(b"\x1b]10;rgb:0101/0202/0303\x07".to_vec()),
            OscAction::WriteToPty(b"\x1b]11;rgb:0404/0505/0606\x07".to_vec()),
        ]
    );
}

#[test]
fn bell_passthrough() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert_eq!(
        protocol.process(b"before\x07after"),
        vec![OscAction::WriteToOuter(vec![0x07])]
    );
}

#[test]
fn bell_flood_is_coalesced_into_one_outer_write() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    let bells = vec![0x07; 8192];

    assert_eq!(
        protocol.process(&bells),
        vec![OscAction::WriteToOuter(bells)]
    );
}

#[test]
fn osc_notifications_accept_9_and_777_with_both_terminators() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert_eq!(
        protocol.process(b"\x1b]9;Build done\x07\x1b]777;notify;Deploy;done\x1b\\"),
        vec![OscAction::WriteToOuter(
            b"\x1b]9;Build done\x07\x1b]777;notify;Deploy;done\x07".to_vec()
        )]
    );
}

#[test]
fn osc_99_is_forwarded_without_interpreting_the_protocol() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert_eq!(
        protocol.process(b"\x1b]99;i=p1.notice:p=title;Hello\x07"),
        vec![OscAction::WriteToOuter(
            b"\x1b]99;i=p1.notice:p=title;Hello\x07".to_vec()
        )]
    );
}

#[test]
fn osc_99_capability_query_is_forwarded_to_the_outer_terminal() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    assert_eq!(
        protocol.process(b"\x1b]99;i=p4q.app:p=?;\x1b\\"),
        vec![OscAction::WriteToOuter(
            b"\x1b]99;i=p4q.app:p=?;\x07".to_vec()
        )]
    );
}

#[test]
fn split_notification_is_reassembled_and_empty_message_is_ignored() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert!(protocol.process(b"\x1b]9;Build").is_empty());
    assert_eq!(
        protocol.process(b" done\x07"),
        vec![OscAction::WriteToOuter(b"\x1b]9;Build done\x07".to_vec())]
    );
    assert!(protocol.process(b"\x1b]777;notify;;\x07").is_empty());
}

#[test]
fn notification_text_is_bounded() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    let mut sequence = b"\x1b]9;".to_vec();
    sequence.extend(vec![b'x'; notification::MAX_NOTIFICATION_FIELD_CHARS + 1]);
    sequence.push(0x07);

    assert_eq!(
        protocol.process(&sequence),
        vec![OscAction::WriteToOuter({
            let mut expected = b"\x1b]9;".to_vec();
            expected.extend(vec![b'x'; notification::MAX_NOTIFICATION_FIELD_CHARS]);
            expected.push(0x07);
            expected
        })]
    );
}

#[test]
fn split_osc_777_accepts_two_full_unicode_fields() {
    let field = "🫑".repeat(notification::MAX_NOTIFICATION_FIELD_CHARS);
    let sequence = format!("\x1b]777;notify;{field};{field}\x07").into_bytes();
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert!(protocol.process(&sequence[..sequence.len() - 1]).is_empty());
    assert_eq!(
        protocol.process(&sequence[sequence.len() - 1..]),
        vec![OscAction::WriteToOuter(sequence)]
    );
}

#[test]
fn unterminated_notification_pending_buffer_is_bounded_separately() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));
    let mut sequence = b"\x1b]9;".to_vec();
    sequence.extend(vec![b'x'; MAX_NOTIFICATION_OSC_BYTES]);

    assert!(protocol.process(&sequence).is_empty());
    assert!(protocol.pending.is_empty());
    assert!(protocol.process(b"ordinary output").is_empty());
}

#[test]
fn notification_control_bytes_are_stripped_before_forwarding() {
    let mut protocol = OscProtocol::new((1, 2, 3), (4, 5, 6));

    assert_eq!(
        protocol.process(b"\x1b]9;safe\x1b[2J-looking\x07"),
        vec![OscAction::WriteToOuter(
            b"\x1b]9;safe[2J-looking\x07".to_vec()
        )]
    );
}
