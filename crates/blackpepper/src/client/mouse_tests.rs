use super::*;
use ratatui::layout::Rect;
use vt100::MouseProtocolMode;

const AREA: Rect = Rect::new(30, 4, 80, 24);

#[test]
fn translates_sgr_coordinates_and_preserves_keyboard_bytes() {
    let mut protocol = MouseInputProtocol::default();
    assert_eq!(
        protocol
            .process(
                b"a\x1b[<64;35;9Mb",
                Some(AREA),
                MouseProtocolEncoding::Sgr,
                MouseProtocolMode::PressRelease,
            )
            .bytes,
        b"a\x1b[<64;5;5Mb"
    );
}

#[test]
fn drops_outside_click_but_clamps_an_active_drag_release() {
    let mut protocol = MouseInputProtocol::default();
    let outside = protocol.process(
        b"\x1b[<0;5;5M",
        Some(AREA),
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::PressRelease,
    );
    assert!(outside.bytes.is_empty());
    assert!(outside.shell_clicked);
    assert_eq!(
        protocol
            .process(
                b"\x1b[<0;35;9M",
                Some(AREA),
                MouseProtocolEncoding::Sgr,
                MouseProtocolMode::PressRelease,
            )
            .bytes,
        b"\x1b[<0;5;5M"
    );
    assert_eq!(
        protocol
            .process(
                b"\x1b[<32;200;99M",
                Some(AREA),
                MouseProtocolEncoding::Sgr,
                MouseProtocolMode::PressRelease,
            )
            .bytes,
        b"\x1b[<32;80;24M"
    );
    assert_eq!(
        protocol
            .process(
                b"\x1b[<0;200;99m",
                Some(AREA),
                MouseProtocolEncoding::Sgr,
                MouseProtocolMode::PressRelease,
            )
            .bytes,
        b"\x1b[<0;80;24m"
    );
}

#[test]
fn joins_split_sgr_sequence_after_its_unambiguous_prefix() {
    let mut protocol = MouseInputProtocol::default();
    assert!(protocol
        .process(
            b"\x1b[<0;35",
            Some(AREA),
            MouseProtocolEncoding::Sgr,
            MouseProtocolMode::PressRelease,
        )
        .bytes
        .is_empty());
    assert_eq!(
        protocol
            .process(
                b";9M",
                Some(AREA),
                MouseProtocolEncoding::Sgr,
                MouseProtocolMode::PressRelease,
            )
            .bytes,
        b"\x1b[<0;5;5M"
    );
}

#[test]
fn drops_mouse_input_for_a_zero_sized_terminal_panel() {
    let mut protocol = MouseInputProtocol::default();
    assert!(protocol
        .process(
            b"\x1b[<0;1;1M",
            Some(Rect::new(0, 0, 0, 0)),
            MouseProtocolEncoding::Sgr,
            MouseProtocolMode::PressRelease,
        )
        .bytes
        .is_empty());
}

#[test]
fn shell_footer_stays_clickable_when_the_child_disabled_mouse_input() {
    let mut protocol = MouseInputProtocol::default();
    let inside = protocol.process(
        b"\x1b[<0;35;9M",
        Some(AREA),
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::None,
    );
    assert!(inside.bytes.is_empty());
    assert!(!inside.shell_clicked);

    let footer = protocol.process(
        b"\x1b[<0;35;29M",
        Some(AREA),
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::None,
    );
    assert!(footer.bytes.is_empty());
    assert!(footer.shell_clicked);
}

#[test]
fn capture_only_mouse_is_not_sent_before_a_viewport_has_rendered() {
    let mut protocol = MouseInputProtocol::default();
    let capture = protocol.process(
        b"\x1b[<0;5;5M",
        None,
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::None,
    );
    assert!(capture.bytes.is_empty());

    let child = protocol.process(
        b"\x1b[<0;5;5M",
        None,
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::PressRelease,
    );
    assert_eq!(child.bytes, b"\x1b[<0;5;5M");
}

#[test]
fn footer_transition_drops_keys_coalesced_into_the_same_terminal_read() {
    let mut protocol = MouseInputProtocol::default();
    let input = protocol.process(
        b"a\x1b[<0;35;29Mb",
        Some(AREA),
        MouseProtocolEncoding::Sgr,
        MouseProtocolMode::None,
    );

    assert!(input.shell_clicked);
    assert!(input.bytes.is_empty());
}
