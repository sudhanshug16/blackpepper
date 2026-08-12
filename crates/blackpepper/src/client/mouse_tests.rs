use super::*;

const AREA: Rect = Rect::new(30, 4, 80, 24);

#[test]
fn translates_sgr_coordinates_and_preserves_keyboard_bytes() {
    let mut protocol = MouseInputProtocol::default();
    assert_eq!(
        protocol.process(b"a\x1b[<64;35;9Mb", Some(AREA), MouseProtocolEncoding::Sgr),
        b"a\x1b[<64;5;5Mb"
    );
}

#[test]
fn drops_outside_click_but_clamps_an_active_drag_release() {
    let mut protocol = MouseInputProtocol::default();
    assert!(protocol
        .process(b"\x1b[<0;5;5M", Some(AREA), MouseProtocolEncoding::Sgr)
        .is_empty());
    assert_eq!(
        protocol.process(b"\x1b[<0;35;9M", Some(AREA), MouseProtocolEncoding::Sgr),
        b"\x1b[<0;5;5M"
    );
    assert_eq!(
        protocol.process(b"\x1b[<32;200;99M", Some(AREA), MouseProtocolEncoding::Sgr),
        b"\x1b[<32;80;24M"
    );
    assert_eq!(
        protocol.process(b"\x1b[<0;200;99m", Some(AREA), MouseProtocolEncoding::Sgr),
        b"\x1b[<0;80;24m"
    );
}

#[test]
fn joins_split_sgr_sequence_after_its_unambiguous_prefix() {
    let mut protocol = MouseInputProtocol::default();
    assert!(protocol
        .process(b"\x1b[<0;35", Some(AREA), MouseProtocolEncoding::Sgr)
        .is_empty());
    assert_eq!(
        protocol.process(b";9M", Some(AREA), MouseProtocolEncoding::Sgr),
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
        )
        .is_empty());
}
