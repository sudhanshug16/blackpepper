use super::glyph::Glyphs;
use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::client_config::ColorTier;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

pub(super) const PEPPERCORN: (u8, u8, u8) = (0xe4, 0x83, 0x4f);
pub(super) const CANVAS: (u8, u8, u8) = (0x1c, 0x1d, 0x1f);
pub(super) const RAISED: (u8, u8, u8) = (0x23, 0x24, 0x27);
pub(super) const INK: (u8, u8, u8) = (0xe6, 0xe4, 0xe1);

/// The neutral ramp between `INK` and `CANVAS`. The design's third grey,
/// `#8b8a87`, styles its own page captions and the mocked terminal body, so it
/// is deliberately absent here — the client draws neither.
const MID: (u8, u8, u8) = (0xb9, 0xb6, 0xb2);
const RECESSIVE: (u8, u8, u8) = (0x6c, 0x6b, 0x68);

/// How far each neutral sits from the foreground, as a percentage blend toward
/// the background. Derived rather than hardcoded so a configured palette keeps
/// its own hue; the defaults reproduce the tokens above exactly.
const MID_BLEND: u16 = 22;
const RECESSIVE_BLEND: u16 = 62;

/// Semantic colors carry meaning, not theme identity, so they are named
/// precisely while colour exists and only fall back to the terminal's own
/// slots at the sixteen-colour floor.
const GREEN: (u8, u8, u8) = (0x98, 0xc3, 0x79);
const YELLOW: (u8, u8, u8) = (0xe5, 0xc0, 0x7b);
const RED: (u8, u8, u8) = (0xe0, 0x6c, 0x75);
const CYAN: (u8, u8, u8) = (0x56, 0xb6, 0xc2);

/// A semantic color and the ANSI slot it degrades into.
#[derive(Clone, Copy)]
struct Semantic {
    exact: (u8, u8, u8),
    ansi: Color,
}

const SEMANTIC_GREEN: Semantic = Semantic {
    exact: GREEN,
    ansi: Color::Green,
};
const SEMANTIC_YELLOW: Semantic = Semantic {
    exact: YELLOW,
    ansi: Color::Yellow,
};
const SEMANTIC_RED: Semantic = Semantic {
    exact: RED,
    ansi: Color::Red,
};
const SEMANTIC_CYAN: Semantic = Semantic {
    exact: CYAN,
    ansi: Color::Cyan,
};

/// Whether the client is painting its own palette or one the user configured.
fn default_palette(state: &ClientState) -> bool {
    state.config.ui.background == CANVAS && state.config.ui.foreground == INK
}

/// One step on the neutral ramp, as an exact color at full depth and the
/// terminal's own grey below it.
fn neutral(state: &ClientState, token: (u8, u8, u8), blend: u16) -> Style {
    let exact = if default_palette(state) {
        token
    } else {
        blend_rgb(
            state.config.ui.foreground,
            state.config.ui.background,
            blend,
        )
    };
    match state.config.ui.color_tier {
        ColorTier::TrueColor | ColorTier::Ansi256 => {
            Style::default().fg(tier_color(exact, state.config.ui.color_tier))
        }
        ColorTier::Ansi16 => Style::default().fg(Color::Gray),
        ColorTier::NoColor => Style::default(),
    }
}

fn semantic(state: &ClientState, color: Semantic) -> Style {
    match state.config.ui.color_tier {
        ColorTier::TrueColor | ColorTier::Ansi256 => {
            Style::default().fg(tier_color(color.exact, state.config.ui.color_tier))
        }
        ColorTier::Ansi16 => Style::default().fg(color.ansi),
        ColorTier::NoColor => Style::default(),
    }
}

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

/// List entries that are present but not the current one.
pub(super) fn mid_style(state: &ClientState) -> Style {
    neutral(state, MID, MID_BLEND)
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

/// The most recessive text: hints, reasons, and anything the eye should skip
/// until it needs it.
pub(super) fn section_style(state: &ClientState) -> Style {
    if state.config.ui.color_tier == ColorTier::NoColor {
        return Style::default().add_modifier(Modifier::DIM);
    }
    neutral(state, RECESSIVE, RECESSIVE_BLEND)
}

pub(super) fn warning_style(state: &ClientState) -> Style {
    semantic(state, SEMANTIC_YELLOW)
}

pub(super) fn danger_style(state: &ClientState) -> Style {
    semantic(state, SEMANTIC_RED)
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

pub(super) fn status_style(state: &ClientState, status: DisplayStatus) -> Style {
    match status {
        // Idle and unsure are absences, so they take the recessive neutral
        // rather than a semantic colour that would claim they mean something.
        DisplayStatus::Idle | DisplayStatus::Ready | DisplayStatus::Unknown => section_style(state),
        DisplayStatus::Working => semantic(state, SEMANTIC_CYAN),
        DisplayStatus::Done => semantic(state, SEMANTIC_GREEN),
        DisplayStatus::NeedsInput => semantic(state, SEMANTIC_YELLOW),
        DisplayStatus::Exited => semantic(state, SEMANTIC_RED),
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
    match connection {
        HostConnection::Local | HostConnection::Connected => semantic(state, SEMANTIC_GREEN),
        HostConnection::Authenticating
        | HostConnection::Reconnecting
        | HostConnection::NeedsAuthentication => semantic(state, SEMANTIC_YELLOW),
        HostConnection::HostKeyBlocked | HostConnection::Failed => semantic(state, SEMANTIC_RED),
        HostConnection::Disconnected => section_style(state),
    }
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
