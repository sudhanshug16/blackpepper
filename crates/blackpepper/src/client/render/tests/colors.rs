use super::{draw, workspace_state};
use crate::client::DisplayStatus;
use crate::client_config::ColorTier;
use ratatui::style::{Color, Modifier};

#[test]
fn supplied_truecolor_tokens_are_exact() {
    let state = workspace_state();
    assert_eq!(
        super::super::style::ui_style(&state).fg,
        Some(Color::Rgb(0xe6, 0xe4, 0xe1))
    );
    assert_eq!(
        super::super::style::ui_style(&state).bg,
        Some(Color::Rgb(0x1c, 0x1d, 0x1f))
    );
    assert_eq!(
        super::super::style::panel_style(&state).bg,
        Some(Color::Rgb(0x23, 0x24, 0x27))
    );
    assert_eq!(
        super::super::style::accent_style(&state).fg,
        Some(Color::Rgb(0xe4, 0x83, 0x4f))
    );
}

#[test]
fn custom_foreground_background_and_derived_surface_are_preserved() {
    let mut state = workspace_state();
    state.config.ui.background = (0x11, 0x22, 0x33);
    state.config.ui.foreground = (0xdd, 0xee, 0xff);

    assert_eq!(
        super::super::style::ui_style(&state).fg,
        Some(Color::Rgb(0xdd, 0xee, 0xff))
    );
    assert_eq!(
        super::super::style::ui_style(&state).bg,
        Some(Color::Rgb(0x11, 0x22, 0x33))
    );
    assert_eq!(
        super::super::style::panel_style(&state).bg,
        Some(Color::Rgb(0x1d, 0x2e, 0x3f))
    );

    let terminal = draw(&mut state, 102, 24);
    assert_eq!(
        terminal.backend().buffer().cell((0, 1)).unwrap().bg,
        Color::Rgb(0x1d, 0x2e, 0x3f)
    );
    assert_eq!(
        terminal.backend().buffer().cell((32, 2)).unwrap().bg,
        Color::Rgb(0x11, 0x22, 0x33)
    );
}

#[test]
fn accent_and_ui_degrade_through_all_color_tiers() {
    let mut state = workspace_state();
    let cases = [
        (ColorTier::TrueColor, Some(Color::Rgb(0xe4, 0x83, 0x4f))),
        (ColorTier::Ansi256, Some(Color::Indexed(173))),
        (ColorTier::Ansi16, Some(Color::Yellow)),
        (ColorTier::NoColor, None),
    ];
    for (tier, accent) in cases {
        state.config.ui.color_tier = tier;
        assert_eq!(super::super::style::accent_style(&state).fg, accent);
    }

    state.config.ui.color_tier = ColorTier::Ansi256;
    assert!(matches!(
        super::super::style::ui_style(&state).fg,
        Some(Color::Indexed(_))
    ));
    assert!(matches!(
        super::super::style::ui_style(&state).bg,
        Some(Color::Indexed(_))
    ));
    state.config.ui.color_tier = ColorTier::Ansi16;
    assert_eq!(super::super::style::ui_style(&state).fg, Some(Color::White));
    assert_eq!(super::super::style::ui_style(&state).bg, Some(Color::Black));
    assert_eq!(
        super::super::style::panel_style(&state).bg,
        Some(Color::Black)
    );
    state.config.ui.color_tier = ColorTier::NoColor;
    assert_eq!(super::super::style::ui_style(&state).fg, None);
    assert_eq!(super::super::style::ui_style(&state).bg, None);
    assert!(super::super::style::accent_style(&state)
        .add_modifier
        .contains(Modifier::BOLD));
}

#[test]
fn every_public_status_has_a_glyph_word_and_semantic_color() {
    let state = workspace_state();
    let cases = [
        (DisplayStatus::Idle, "· idle", Color::DarkGray),
        (DisplayStatus::Working, "▸ running", Color::Cyan),
        (DisplayStatus::NeedsInput, "! asks", Color::Yellow),
        (DisplayStatus::Done, "✓ done", Color::Green),
        (DisplayStatus::Exited, "× exited", Color::Red),
        (DisplayStatus::Unknown, "? unsure", Color::DarkGray),
    ];
    for (status, expected, color) in cases {
        let span = super::super::style::status_span(&state, status, None);
        assert_eq!(span.content.as_ref(), expected);
        assert_eq!(span.style.fg, Some(color));
    }
}

#[test]
fn no_color_keeps_status_words_and_reverse_video_selection() {
    let mut state = workspace_state();
    state.config.ui.color_tier = ColorTier::NoColor;
    let status = super::super::style::status_span(&state, DisplayStatus::NeedsInput, None);
    assert_eq!(status.content.as_ref(), "! asks");
    assert_eq!(status.style.fg, None);
    assert!(super::super::style::selected_style(&state)
        .add_modifier
        .contains(Modifier::REVERSED));
}
