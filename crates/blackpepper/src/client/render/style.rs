use super::glyph::Glyphs;
use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::client_config::ColorTier;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub(super) const PEPPERCORN: (u8, u8, u8) = (0xe4, 0x83, 0x4f);
pub(super) const CANVAS: (u8, u8, u8) = (0x1c, 0x1d, 0x1f);
pub(super) const RAISED: (u8, u8, u8) = (0x23, 0x24, 0x27);
pub(super) const INK: (u8, u8, u8) = (0xe6, 0xe4, 0xe1);

pub(super) fn ui_style(state: &ClientState) -> Style {
    style_for_pair(
        state.config.ui.foreground,
        state.config.ui.background,
        state.config.ui.color_tier,
    )
}

/// Raised surfaces use the supplied token for the default palette. A custom
/// terminal palette keeps its configured colors and derives the same 6% tier.
pub(super) fn panel_style(state: &ClientState) -> Style {
    let raised = if state.config.ui.background == CANVAS && state.config.ui.foreground == INK {
        RAISED
    } else {
        blend_rgb(state.config.ui.background, state.config.ui.foreground, 6)
    };
    let background = match state.config.ui.color_tier {
        ColorTier::TrueColor | ColorTier::Ansi256 => raised,
        ColorTier::Ansi16 | ColorTier::NoColor => state.config.ui.background,
    };
    style_for_pair(
        state.config.ui.foreground,
        background,
        state.config.ui.color_tier,
    )
}

/// Mid-weight text: list entries that are present but not the current one.
/// Derived from the palette so a custom foreground keeps its own hue, and one
/// step short of `section_style` so the two stay distinguishable.
pub(super) fn mid_style(state: &ClientState) -> Style {
    let mid = if state.config.ui.background == CANVAS && state.config.ui.foreground == INK {
        (0xb9, 0xb6, 0xb2)
    } else {
        blend_rgb(state.config.ui.foreground, state.config.ui.background, 22)
    };
    match state.config.ui.color_tier {
        ColorTier::NoColor => Style::default(),
        tier => Style::default().fg(tier_color(mid, tier)),
    }
}

/// The selected row. The design paints it with the accent and also asks for
/// `Modifier::REVERSED`; reversing an accent foreground satisfies both, giving
/// an accent fill where colour exists and plain reverse video where it does
/// not. REVERSED is set in every tier so selection survives the 2-colour floor.
pub(super) fn selected_style(state: &ClientState) -> Style {
    match state.config.ui.color_tier {
        ColorTier::TrueColor => Style::default()
            .fg(Color::Rgb(PEPPERCORN.0, PEPPERCORN.1, PEPPERCORN.2))
            .bg(tier_color(state.config.ui.background, ColorTier::TrueColor))
            .add_modifier(Modifier::REVERSED),
        ColorTier::Ansi256 => Style::default()
            .fg(Color::Indexed(173))
            .bg(tier_color(state.config.ui.background, ColorTier::Ansi256))
            .add_modifier(Modifier::REVERSED),
        ColorTier::Ansi16 | ColorTier::NoColor => ui_style(state).add_modifier(Modifier::REVERSED),
    }
}

pub(super) fn accent_style(state: &ClientState) -> Style {
    match state.config.ui.color_tier {
        ColorTier::TrueColor => {
            Style::default().fg(Color::Rgb(PEPPERCORN.0, PEPPERCORN.1, PEPPERCORN.2))
        }
        ColorTier::Ansi256 => Style::default().fg(Color::Indexed(173)),
        ColorTier::Ansi16 => Style::default().fg(Color::Yellow),
        ColorTier::NoColor => Style::default().add_modifier(Modifier::BOLD),
    }
}

/// The `bp` anchor. It is the one fixed point on both status rows, so it is
/// drawn bold as well as accented and stays legible when accent degrades to
/// bold alone.
pub(super) fn anchor_style(state: &ClientState) -> Style {
    accent_style(state).add_modifier(Modifier::BOLD)
}

/// The mode badge and the `:approve` action. At sixteen colours the design
/// drops the accent fill for reverse video rather than betting on a yellow
/// that the user's theme may have redefined into something unreadable.
pub(super) fn accent_badge_style(state: &ClientState) -> Style {
    match state.config.ui.color_tier {
        ColorTier::Ansi16 | ColorTier::NoColor => {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        }
        tier => Style::default()
            .fg(tier_color(state.config.ui.background, tier))
            .bg(match tier {
                ColorTier::Ansi256 => Color::Indexed(173),
                _ => Color::Rgb(PEPPERCORN.0, PEPPERCORN.1, PEPPERCORN.2),
            })
            .add_modifier(Modifier::BOLD),
    }
}

pub(super) fn section_style(state: &ClientState) -> Style {
    match state.config.ui.color_tier {
        ColorTier::NoColor => Style::default().add_modifier(Modifier::DIM),
        _ => Style::default().fg(Color::DarkGray),
    }
}

pub(super) fn warning_style(state: &ClientState) -> Style {
    semantic_style(state, Color::Yellow)
}

pub(super) fn danger_style(state: &ClientState) -> Style {
    semantic_style(state, Color::Red)
}

/// `glyph word`, or `glyph <detail>` when the caller has something more
/// specific to say than the vocabulary word — the design gives a running agent
/// its elapsed time in that column rather than repeating "running".
pub(super) fn status_text(
    state: &ClientState,
    status: DisplayStatus,
    detail: Option<&str>,
) -> String {
    let glyph = Glyphs::of(state).status(status);
    let tail = detail.unwrap_or_else(|| status.public_word());
    format!("{glyph} {tail}")
}

pub(super) fn status_color(status: DisplayStatus) -> Color {
    match status {
        DisplayStatus::Idle | DisplayStatus::Ready | DisplayStatus::Unknown => Color::DarkGray,
        DisplayStatus::Working => Color::Cyan,
        DisplayStatus::Done => Color::Green,
        DisplayStatus::NeedsInput => Color::Yellow,
        DisplayStatus::Exited => Color::Red,
    }
}

pub(super) fn status_span(
    state: &ClientState,
    status: DisplayStatus,
    detail: Option<&str>,
) -> Span<'static> {
    Span::styled(
        status_text(state, status, detail),
        status_style(state, status),
    )
}

pub(super) fn status_style(state: &ClientState, status: DisplayStatus) -> Style {
    semantic_style(state, status_color(status))
}

/// The status column inside a dense list. An idle row spends one cell on its
/// glyph rather than six on `· idle`, because in a column of rows the absence
/// of activity is what the glyph alone already says. Rows with something to
/// report keep their full text.
pub(super) fn list_status_text(
    state: &ClientState,
    status: DisplayStatus,
    detail: Option<&str>,
) -> String {
    if detail.is_none() && matches!(status, DisplayStatus::Idle | DisplayStatus::Ready) {
        return Glyphs::of(state).status(status).to_owned();
    }
    status_text(state, status, detail)
}

pub(super) fn list_status_span(
    state: &ClientState,
    status: DisplayStatus,
    detail: Option<&str>,
) -> Span<'static> {
    Span::styled(
        list_status_text(state, status, detail),
        status_style(state, status),
    )
}

/// Host connection colors reuse the agent-status palette, so green/yellow/red
/// mean the same severity in both columns even though the vocabularies differ.
pub(super) fn connection_style(state: &ClientState, connection: HostConnection) -> Style {
    if state.config.ui.color_tier == ColorTier::NoColor {
        return Style::default();
    }
    let color = match connection {
        HostConnection::Local | HostConnection::Connected => Color::Green,
        HostConnection::Authenticating
        | HostConnection::Reconnecting
        | HostConnection::NeedsAuthentication => Color::Yellow,
        HostConnection::HostKeyBlocked | HostConnection::Failed => Color::Red,
        HostConnection::Disconnected => Color::DarkGray,
    };
    Style::default().fg(color)
}

fn style_for_pair(foreground: (u8, u8, u8), background: (u8, u8, u8), tier: ColorTier) -> Style {
    if tier == ColorTier::NoColor {
        Style::default()
    } else {
        Style::default()
            .fg(tier_color(foreground, tier))
            .bg(tier_color(background, tier))
    }
}

fn semantic_style(state: &ClientState, color: Color) -> Style {
    if state.config.ui.color_tier == ColorTier::NoColor {
        Style::default()
    } else {
        Style::default().fg(color)
    }
}

fn tier_color(rgb: (u8, u8, u8), tier: ColorTier) -> Color {
    match tier {
        ColorTier::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
        ColorTier::Ansi256 => Color::Indexed(nearest_ansi256(rgb)),
        ColorTier::Ansi16 => nearest_ansi16(rgb),
        ColorTier::NoColor => Color::Reset,
    }
}

fn blend_rgb(base: (u8, u8, u8), toward: (u8, u8, u8), percent: u16) -> (u8, u8, u8) {
    let blend = |base: u8, toward: u8| {
        ((u16::from(base) * (100 - percent) + u16::from(toward) * percent) / 100) as u8
    };
    (
        blend(base.0, toward.0),
        blend(base.1, toward.1),
        blend(base.2, toward.2),
    )
}

fn nearest_ansi256(rgb: (u8, u8, u8)) -> u8 {
    (16_u8..=255)
        .min_by_key(|index| color_distance(rgb, ansi256_rgb(*index)))
        .unwrap_or(16)
}

fn ansi256_rgb(index: u8) -> (u8, u8, u8) {
    if index >= 232 {
        let shade = 8 + (index - 232) * 10;
        return (shade, shade, shade);
    }
    let cube = index - 16;
    let levels = [0, 95, 135, 175, 215, 255];
    (
        levels[usize::from(cube / 36)],
        levels[usize::from((cube % 36) / 6)],
        levels[usize::from(cube % 6)],
    )
}

fn nearest_ansi16(rgb: (u8, u8, u8)) -> Color {
    const COLORS: [(Color, (u8, u8, u8)); 16] = [
        (Color::Black, (0, 0, 0)),
        (Color::Red, (128, 0, 0)),
        (Color::Green, (0, 128, 0)),
        (Color::Yellow, (128, 128, 0)),
        (Color::Blue, (0, 0, 128)),
        (Color::Magenta, (128, 0, 128)),
        (Color::Cyan, (0, 128, 128)),
        (Color::Gray, (192, 192, 192)),
        (Color::DarkGray, (128, 128, 128)),
        (Color::LightRed, (255, 0, 0)),
        (Color::LightGreen, (0, 255, 0)),
        (Color::LightYellow, (255, 255, 0)),
        (Color::LightBlue, (0, 0, 255)),
        (Color::LightMagenta, (255, 0, 255)),
        (Color::LightCyan, (0, 255, 255)),
        (Color::White, (255, 255, 255)),
    ];
    COLORS
        .into_iter()
        .min_by_key(|(_, candidate)| color_distance(rgb, *candidate))
        .map(|(color, _)| color)
        .unwrap_or(Color::Reset)
}

fn color_distance(left: (u8, u8, u8), right: (u8, u8, u8)) -> u32 {
    let channel = |left: u8, right: u8| i32::from(left) - i32::from(right);
    [
        channel(left.0, right.0),
        channel(left.1, right.1),
        channel(left.2, right.2),
    ]
    .into_iter()
    .map(|value| value.unsigned_abs().pow(2))
    .sum()
}
