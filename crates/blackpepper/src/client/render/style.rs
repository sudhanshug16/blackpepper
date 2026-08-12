use super::glyph::Glyphs;
use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::client_config::ColorTier;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// The active palette. Every colour below comes from here, so a theme change
/// is one lookup rather than a sweep through the renderer.
fn theme(state: &ClientState) -> crate::client_config::Theme {
    state.config.ui.theme
}

/// How far each neutral sits from the foreground, as a percentage blend toward
/// the background. Used only when the user has overridden the surfaces, in
/// which case the theme's own neutrals no longer sit on the right ramp.
const MID_BLEND: u16 = 22;
const RECESSIVE_BLEND: u16 = 62;

/// Whether the client is painting the theme's own surfaces or ones the user
/// pinned in config.
fn themed_surfaces(state: &ClientState) -> bool {
    let theme = theme(state);
    state.config.ui.background == theme.canvas && state.config.ui.foreground == theme.ink
}

/// One step on the neutral ramp: the theme's value when the theme owns the
/// surfaces, otherwise derived from whatever the user pinned.
fn neutral(state: &ClientState, token: (u8, u8, u8), blend: u16) -> Style {
    let exact = if themed_surfaces(state) {
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

/// A semantic colour: exact while colour exists, the terminal's own slot at
/// the sixteen-colour floor.
fn semantic(state: &ClientState, exact: (u8, u8, u8), slot: Color) -> Style {
    match state.config.ui.color_tier {
        ColorTier::TrueColor | ColorTier::Ansi256 => {
            Style::default().fg(tier_color(exact, state.config.ui.color_tier))
        }
        ColorTier::Ansi16 => Style::default().fg(slot),
        ColorTier::NoColor => Style::default(),
    }
}

fn slot_color(slot: crate::client_config::AnsiSlot) -> Color {
    use crate::client_config::AnsiSlot;
    match slot {
        AnsiSlot::Magenta => Color::Magenta,
        AnsiSlot::Blue => Color::Blue,
        AnsiSlot::Yellow => Color::Yellow,
        AnsiSlot::Cyan => Color::Cyan,
        AnsiSlot::Green => Color::Green,
        AnsiSlot::Red => Color::Red,
    }
}

pub(super) fn ui_style(state: &ClientState) -> Style {
    style_for_pair(
        state.config.ui.foreground,
        state.config.ui.background,
        state.config.ui.color_tier,
    )
}

/// Raised surfaces use the theme's own token. A pinned background keeps its
/// colour and derives the same 6% tier.
pub(super) fn panel_style(state: &ClientState) -> Style {
    let raised = if themed_surfaces(state) {
        theme(state).raised
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
    neutral(state, theme(state).mid, MID_BLEND)
}

/// The accent as a foreground colour, or `None` when this tier and theme have
/// no hue to spend — a theme with no accent at all, or one whose hue has
/// nowhere safe to land at the sixteen-colour floor.
fn accent_color(state: &ClientState) -> Option<Color> {
    let theme = theme(state);
    let accent = theme.accent?;
    match state.config.ui.color_tier {
        ColorTier::TrueColor | ColorTier::Ansi256 => {
            Some(tier_color(accent, state.config.ui.color_tier))
        }
        ColorTier::Ansi16 => match theme.accent_fallback {
            crate::client_config::AccentFallback::Slot(slot) => Some(slot_color(slot)),
            crate::client_config::AccentFallback::Reverse => None,
        },
        ColorTier::NoColor => None,
    }
}

/// The selected row. The design paints it with the accent and also asks for
/// `Modifier::REVERSED`; reversing an accent foreground satisfies both, giving
/// an accent fill where colour exists and plain reverse video where it does
/// not. REVERSED is set in every tier so selection survives the 2-colour floor.
pub(super) fn selected_style(state: &ClientState) -> Style {
    match accent_color(state) {
        Some(accent) => Style::default()
            .fg(accent)
            .bg(tier_color(
                state.config.ui.background,
                state.config.ui.color_tier,
            ))
            .add_modifier(Modifier::REVERSED),
        None => ui_style(state).add_modifier(Modifier::REVERSED),
    }
}

/// The accent, wherever a single glyph or word carries the brand rather than a
/// state. Without a hue it falls back to bold, which is the only emphasis a
/// two-colour terminal has.
pub(super) fn accent_style(state: &ClientState) -> Style {
    match accent_color(state) {
        Some(accent) => Style::default().fg(accent),
        None => Style::default().add_modifier(Modifier::BOLD),
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
    // The badge is a filled block, so at the floor it takes reverse video
    // rather than betting on a slot the user's scheme may have redefined.
    match (state.config.ui.color_tier, accent_color(state)) {
        (ColorTier::TrueColor | ColorTier::Ansi256, Some(accent)) => Style::default()
            .fg(tier_color(
                state.config.ui.background,
                state.config.ui.color_tier,
            ))
            .bg(accent)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED),
    }
}

/// The most recessive text: hints, reasons, and anything the eye should skip
/// until it needs it.
pub(super) fn section_style(state: &ClientState) -> Style {
    if state.config.ui.color_tier == ColorTier::NoColor {
        return Style::default().add_modifier(Modifier::DIM);
    }
    neutral(state, theme(state).recessive, RECESSIVE_BLEND)
}

pub(super) fn warning_style(state: &ClientState) -> Style {
    semantic(state, theme(state).yellow, Color::Yellow)
}

pub(super) fn danger_style(state: &ClientState) -> Style {
    semantic(state, theme(state).red, Color::Red)
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
        DisplayStatus::Working => semantic(state, theme(state).cyan, Color::Cyan),
        DisplayStatus::Done => semantic(state, theme(state).green, Color::Green),
        DisplayStatus::NeedsInput => semantic(state, theme(state).yellow, Color::Yellow),
        DisplayStatus::Exited => semantic(state, theme(state).red, Color::Red),
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
        HostConnection::Local | HostConnection::Connected => {
            semantic(state, theme(state).green, Color::Green)
        }
        HostConnection::Authenticating
        | HostConnection::Reconnecting
        | HostConnection::NeedsAuthentication => {
            semantic(state, theme(state).yellow, Color::Yellow)
        }
        HostConnection::HostKeyBlocked | HostConnection::Failed => {
            semantic(state, theme(state).red, Color::Red)
        }
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
