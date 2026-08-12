//! The palettes the client can paint in.
//!
//! A theme is mostly one decision — the accent hue — plus the surface polarity
//! that hue was chosen against. Everything else is the shared vocabulary: the
//! same four status colours, the same neutral ramp, the same layout.
//!
//! The accent's sixteen-colour fallback is part of the theme rather than a
//! detail of rendering, because that is where a hue can collide with meaning.
//! The status vocabulary already spends cyan, yellow, green and red, so an
//! accent that degrades into one of those makes the brand mark and a real
//! alert the same colour on a terminal that cannot tell them apart. Themes
//! whose hue has nowhere safe to land degrade to reverse video instead, which
//! is unambiguous on every terminal ever made.

/// One of the sixteen colours every terminal has, named rather than numbered
/// so the user's own scheme decides what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnsiSlot {
    Magenta,
    Blue,
    Yellow,
    Cyan,
    Green,
    Red,
}

/// What the accent becomes once exact colour is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentFallback {
    /// A named slot. Only valid when no status colour uses the same slot.
    Slot(AnsiSlot),
    /// Reverse video. Always unambiguous, at the cost of the hue.
    Reverse,
}

/// The slots the status vocabulary has already claimed. An accent may not
/// degrade into any of these.
pub const STATUS_SLOTS: [AnsiSlot; 4] = [
    AnsiSlot::Cyan,
    AnsiSlot::Yellow,
    AnsiSlot::Green,
    AnsiSlot::Red,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    /// One line, shown by `:theme`.
    pub summary: &'static str,
    pub canvas: (u8, u8, u8),
    pub raised: (u8, u8, u8),
    pub ink: (u8, u8, u8),
    pub mid: (u8, u8, u8),
    pub recessive: (u8, u8, u8),
    /// `None` paints no accent at all: the four accent sites become reverse
    /// video and colour is left to mean state and nothing else.
    pub accent: Option<(u8, u8, u8)>,
    pub accent_fallback: AccentFallback,
    pub green: (u8, u8, u8),
    pub yellow: (u8, u8, u8),
    pub red: (u8, u8, u8),
    pub cyan: (u8, u8, u8),
}

/// Shared dark surfaces and status colours. Six of the seven themes differ
/// only in their accent, so the rest is written once.
const DARK_CANVAS: (u8, u8, u8) = (0x1c, 0x1d, 0x1f);
const DARK_RAISED: (u8, u8, u8) = (0x23, 0x24, 0x27);
const DARK_INK: (u8, u8, u8) = (0xe6, 0xe4, 0xe1);
const DARK_MID: (u8, u8, u8) = (0xb9, 0xb6, 0xb2);
const DARK_RECESSIVE: (u8, u8, u8) = (0x6c, 0x6b, 0x68);
const DARK_GREEN: (u8, u8, u8) = (0x98, 0xc3, 0x79);
const DARK_YELLOW: (u8, u8, u8) = (0xe5, 0xc0, 0x7b);
const DARK_RED: (u8, u8, u8) = (0xe0, 0x6c, 0x75);
const DARK_CYAN: (u8, u8, u8) = (0x56, 0xb6, 0xc2);

const fn dark(
    name: &'static str,
    summary: &'static str,
    accent: Option<(u8, u8, u8)>,
    accent_fallback: AccentFallback,
) -> Theme {
    Theme {
        name,
        summary,
        canvas: DARK_CANVAS,
        raised: DARK_RAISED,
        ink: DARK_INK,
        mid: DARK_MID,
        recessive: DARK_RECESSIVE,
        accent,
        accent_fallback,
        green: DARK_GREEN,
        yellow: DARK_YELLOW,
        red: DARK_RED,
        cyan: DARK_CYAN,
    }
}

/// Every theme, in the order `:theme` lists them. The first is the default.
pub const THEMES: [Theme; 7] = [
    dark(
        "brass",
        "muted warmth, no LLM-orange association",
        Some((0xb8, 0xa0, 0x4a)),
        // Brass sits where ANSI yellow does, and yellow already means "asks".
        // Reverse video keeps the mark distinct from an alert at the floor.
        AccentFallback::Reverse,
    ),
    dark(
        "violet",
        "the only hue the status vocabulary does not use",
        Some((0xc7, 0x7d, 0xff)),
        AccentFallback::Slot(AnsiSlot::Magenta),
    ),
    dark(
        "pink",
        "softer than violet, same magenta underneath",
        Some((0xd4, 0x78, 0x8c)),
        AccentFallback::Slot(AnsiSlot::Magenta),
    ),
    dark(
        "peppercorn",
        "the original ember",
        Some((0xe4, 0x83, 0x4f)),
        // The ember has brass's problem for the same reason.
        AccentFallback::Reverse,
    ),
    dark(
        "indigo",
        "calmest hue, but dark blue renders badly on many terminals",
        Some((0x7f, 0x8c, 0xff)),
        AccentFallback::Slot(AnsiSlot::Blue),
    ),
    dark(
        "none",
        "no accent: colour means state and nothing else",
        None,
        AccentFallback::Reverse,
    ),
    Theme {
        name: "violet-light",
        summary: "violet for a light terminal background",
        canvas: (0xfa, 0xf9, 0xf7),
        raised: (0xec, 0xea, 0xe6),
        ink: (0x1c, 0x1d, 0x1f),
        mid: (0x4a, 0x4a, 0x48),
        recessive: (0x6c, 0x6b, 0x68),
        accent: Some((0x8b, 0x2f, 0xd6)),
        accent_fallback: AccentFallback::Slot(AnsiSlot::Magenta),
        green: (0x3f, 0x8c, 0x3f),
        yellow: (0x9a, 0x6b, 0x00),
        // The specimen shows no `exited` or `running` row, so these two are
        // darkened to the same lightness as the green and yellow it does show.
        red: (0xb0, 0x2a, 0x2a),
        cyan: (0x10, 0x6b, 0x78),
    },
];

pub const DEFAULT_THEME: &str = THEMES[0].name;

pub fn by_name(name: &str) -> Option<Theme> {
    let name = name.trim().to_ascii_lowercase();
    THEMES.into_iter().find(|theme| theme.name == name)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    THEMES.iter().map(|theme| theme.name)
}

impl Theme {
    /// True when the theme paints on a light surface, which flips what
    /// "raised" and "recessive" have to do to stay legible.
    pub fn is_light(&self) -> bool {
        let (red, green, blue) = self.canvas;
        u16::from(red) + u16::from(green) + u16::from(blue) > 3 * 128
    }
}

#[cfg(test)]
mod tests {
    use super::{by_name, names, AccentFallback, Theme, DEFAULT_THEME, STATUS_SLOTS, THEMES};

    #[test]
    fn the_default_theme_exists_and_is_first() {
        assert_eq!(THEMES[0].name, DEFAULT_THEME);
        assert!(by_name(DEFAULT_THEME).is_some());
        assert_eq!(DEFAULT_THEME, "brass");
    }

    #[test]
    fn theme_names_are_unique_and_lookup_is_case_insensitive() {
        let mut sorted = names().collect::<Vec<_>>();
        sorted.sort_unstable();
        let count = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), count, "duplicate theme name");
        assert_eq!(by_name("BRASS").map(|t| t.name), Some("brass"));
        assert_eq!(by_name("  violet  ").map(|t| t.name), Some("violet"));
        assert!(by_name("nonesuch").is_none());
    }

    /// The rule the design's own analysis turns on: an accent that degrades
    /// into a status colour makes the brand mark and a real alert
    /// indistinguishable on a sixteen-colour terminal.
    #[test]
    fn no_accent_degrades_into_a_colour_that_already_means_something() {
        for theme in THEMES {
            if let AccentFallback::Slot(slot) = theme.accent_fallback {
                assert!(
                    !STATUS_SLOTS.contains(&slot),
                    "{} degrades to {slot:?}, which the status vocabulary already uses",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn a_theme_without_an_accent_cannot_claim_a_slot() {
        for theme in THEMES.into_iter().filter(|theme| theme.accent.is_none()) {
            assert_eq!(theme.accent_fallback, AccentFallback::Reverse);
        }
    }

    #[test]
    fn only_the_light_theme_reports_a_light_surface() {
        let light = THEMES
            .into_iter()
            .filter(Theme::is_light)
            .map(|theme| theme.name)
            .collect::<Vec<_>>();
        assert_eq!(light, ["violet-light"]);
    }
}
