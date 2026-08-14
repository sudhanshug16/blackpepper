use super::*;
use std::io::Cursor;
use std::sync::mpsc;

#[test]
fn reader_tags_output_and_exit_with_the_same_attachment() {
    let workspace_id = WorkspaceId::new();
    let attachment_id = uuid::Uuid::new_v4();
    let (sender, receiver) = mpsc::channel();
    let mut bytes = Cursor::new(b"current attachment".to_vec());

    read_output(workspace_id, attachment_id, &mut bytes, sender);

    assert!(matches!(
        receiver.recv().unwrap(),
        ClientEvent::TerminalOutput(workspace, attachment, payload)
            if workspace == workspace_id
                && attachment == attachment_id
                && payload == b"current attachment"
    ));
    assert!(matches!(
        receiver.recv().unwrap(),
        ClientEvent::TerminalExited(workspace, attachment)
            if workspace == workspace_id && attachment == attachment_id
    ));
}

#[test]
fn clipboard_outer_handoff_is_normalized_and_concise_without_text_evidence() {
    let mut outer = Vec::new();
    let notice = dispatch_clipboard(
        ClipboardTarget::System,
        "private clipboard text",
        |_, _| Err("headless display".to_string()),
        |sequence| {
            outer.extend_from_slice(sequence);
            Ok(())
        },
    )
    .expect("outer handoff should be visible");

    assert_eq!(outer, b"\x1b]52;c;cHJpdmF0ZSBjbGlwYm9hcmQgdGV4dA==\x07");
    assert_eq!(notice, "Copy sent to your terminal.");
    assert!(!notice.contains("private clipboard text"));
}

#[test]
fn clipboard_success_still_reaches_the_outer_terminal_for_browser_clients() {
    let mut outer = Vec::new();
    let notice = dispatch_clipboard(
        ClipboardTarget::System,
        "browser copy",
        |_, _| Ok(()),
        |sequence| {
            outer.extend_from_slice(sequence);
            Ok(())
        },
    );

    assert_eq!(notice.as_deref(), Some("Copied."));
    assert_eq!(outer, b"\x1b]52;c;YnJvd3NlciBjb3B5\x07");
}

#[test]
fn native_clipboard_success_wins_when_outer_handoff_fails() {
    let notice = dispatch_clipboard(
        ClipboardTarget::System,
        "copy",
        |_, _| Ok(()),
        |_| Err("outer closed".to_owned()),
    );

    assert_eq!(notice.as_deref(), Some("Copied."));
}

#[test]
fn clipboard_double_failure_is_actionable() {
    let notice = dispatch_clipboard(
        ClipboardTarget::System,
        "copy",
        |_, _| Err("native denied".to_string()),
        |_| Err("outer closed".to_string()),
    )
    .expect("copy failure should be visible");

    assert_eq!(notice, "Copy failed.");
}

#[test]
fn primary_clipboard_choice_reaches_native_and_outer_sinks() {
    let mut native_target = None;
    let mut outer = Vec::new();

    let notice = dispatch_clipboard(
        ClipboardTarget::Primary,
        "primary copy",
        |target, _| {
            native_target = Some(target);
            Ok(())
        },
        |sequence| {
            outer.extend_from_slice(sequence);
            Ok(())
        },
    );

    assert_eq!(notice.as_deref(), Some("Copied."));
    assert_eq!(native_target, Some(ClipboardTarget::Primary));
    assert_eq!(outer, b"\x1b]52;p;cHJpbWFyeSBjb3B5\x07");
}
