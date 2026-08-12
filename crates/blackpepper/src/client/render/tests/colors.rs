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
    // Brass is the default palette.
    assert_eq!(
        super::super::style::accent_style(&state).fg,
        Some(Color::Rgb(0xb8, 0xa0, 0x4a))
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
    // Brass has no safe slot at sixteen colours — ANSI yellow already means
    // "asks" — so it gives up the hue rather than the distinction.
    let cases = [
        (ColorTier::TrueColor, Some(Color::Rgb(0xb8, 0xa0, 0x4a))),
        (ColorTier::Ansi256, Some(Color::Indexed(143))),
        (ColorTier::Ansi16, None),
        (ColorTier::NoColor, None),
    ];
    for (tier, accent) in cases {
        state.config.ui.color_tier = tier;
        assert_eq!(
            super::super::style::accent_style(&state).fg,
            accent,
            "brass at {tier:?}"
        );
    }
    // A theme whose hue does have a free slot keeps it all the way down.
    state.config.ui.theme = crate::client_config::theme::by_name("violet").unwrap();
    state.config.ui.color_tier = ColorTier::Ansi16;
    assert_eq!(
        super::super::style::accent_style(&state).fg,
        Some(Color::Magenta)
    );
    state.config.ui.theme = crate::client_config::theme::THEMES[0];

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

/// At full colour depth every status names its colour exactly, so the client
/// looks the same whatever the terminal theme is.
#[test]
fn every_public_status_has_a_glyph_word_and_an_exact_color() {
    let state = workspace_state();
    let cases = [
        (DisplayStatus::Idle, "· idle", Color::Rgb(0x6c, 0x6b, 0x68)),
        (
            DisplayStatus::Working,
            "▸ running",
            Color::Rgb(0x56, 0xb6, 0xc2),
        ),
        (
            DisplayStatus::NeedsInput,
            "! asks",
            Color::Rgb(0xe5, 0xc0, 0x7b),
        ),
        (DisplayStatus::Done, "✓ done", Color::Rgb(0x98, 0xc3, 0x79)),
        (
            DisplayStatus::Exited,
            "× exited",
            Color::Rgb(0xe0, 0x6c, 0x75),
        ),
        (
            DisplayStatus::Unknown,
            "? unsure",
            Color::Rgb(0x6c, 0x6b, 0x68),
        ),
    ];
    for (status, expected, color) in cases {
        let span = super::super::style::status_span(&state, status, None);
        assert_eq!(span.content.as_ref(), expected);
        assert_eq!(span.style.fg, Some(color));
    }
}

/// Only at the sixteen-colour floor does the client hand its palette over to
/// the terminal's own slots.
#[test]
fn the_sixteen_color_floor_falls_back_to_the_terminals_own_slots() {
    let mut state = workspace_state();
    state.config.ui.color_tier = ColorTier::Ansi16;
    let cases = [
        (DisplayStatus::Working, Color::Cyan),
        (DisplayStatus::NeedsInput, Color::Yellow),
        (DisplayStatus::Done, Color::Green),
        (DisplayStatus::Exited, Color::Red),
        (DisplayStatus::Idle, Color::Gray),
    ];
    for (status, color) in cases {
        assert_eq!(
            super::super::style::status_span(&state, status, None)
                .style
                .fg,
            Some(color),
            "{status:?} did not degrade to a named slot"
        );
    }
}

/// The neutral ramp is derived, so a configured palette keeps its own hue
/// instead of being overpainted with the design's greys.
#[test]
fn a_custom_palette_derives_its_own_neutrals() {
    let mut state = workspace_state();
    state.config.ui.foreground = (0xff, 0xff, 0xff);
    state.config.ui.background = (0x00, 0x00, 0x40);
    let Some(Color::Rgb(red, green, blue)) = super::super::style::section_style(&state).fg else {
        panic!("recessive neutral lost its exact colour");
    };
    assert!(
        red == green && blue > red,
        "derived neutral dropped the palette's blue cast: {red},{green},{blue}"
    );
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
