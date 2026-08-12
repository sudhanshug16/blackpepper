//! The complete glyph budget and its ASCII fallback.
//!
//! Every marker the client draws is named here. Nothing else in the renderer
//! may emit one literally, so `ui.glyphs = "ascii"` is a total switch rather
//! than a best-effort one. The four-row brand mark is the deliberate exception:
//! it is built from half-blocks and has no ASCII form, which is why the design
//! states its own font requirement. Every status and connection marker is
//! one column wide in both repertoires, so the fixed column arithmetic in the
//! sidebar and ports panel is identical whichever set is active.

use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::client_config::GlyphSet;

/// Braille spinner phases, in rotation order.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Copy)]
pub(super) struct Glyphs(GlyphSet);

impl Glyphs {
    pub(super) fn of(state: &ClientState) -> Self {
        Self(state.config.ui.glyphs)
    }

    fn pick(self, unicode: &'static str, ascii: &'static str) -> &'static str {
        match self.0 {
            GlyphSet::Unicode => unicode,
            GlyphSet::Ascii => ascii,
        }
    }

    /// Host reachable, or the local machine.
    pub(super) fn connected(self) -> &'static str {
        self.pick("●", "*")
    }

    /// Host deliberately not connected.
    pub(super) fn disconnected(self) -> &'static str {
        self.pick("○", "o")
    }

    /// Host mid-transition: connecting, reconnecting, or awaiting credentials.
    pub(super) fn transitional(self) -> &'static str {
        self.pick("◐", ".")
    }

    /// Agent is running.
    pub(super) fn running(self) -> &'static str {
        self.pick("▸", ">")
    }

    /// Agent is idle, or a column has nothing to report.
    pub(super) fn idle(self) -> &'static str {
        self.pick("·", ".")
    }

    /// Agent finished.
    pub(super) fn done(self) -> &'static str {
        self.pick("✓", "+")
    }

    /// Agent process is gone.
    pub(super) fn exited(self) -> &'static str {
        self.pick("×", "x")
    }

    /// Something needs a person, or a warning banner.
    pub(super) fn attention(self) -> &'static str {
        "!"
    }

    /// Coverage is partial and the client will not guess.
    pub(super) fn unsure(self) -> &'static str {
        "?"
    }

    /// A warning that owns its own row.
    pub(super) fn warning(self) -> &'static str {
        self.pick("⚠", "!")
    }

    /// Work is in flight on a host. `phase` advances once per tick.
    pub(super) fn spinner(self, phase: usize) -> &'static str {
        match self.0 {
            GlyphSet::Unicode => SPINNER[phase % SPINNER.len()],
            GlyphSet::Ascii => "-",
        }
    }

    /// Forward source to destination.
    pub(super) fn arrow(self) -> &'static str {
        self.pick("→", ">")
    }

    /// Vertical movement hint, as used in footer and overlay affordances.
    pub(super) fn updown(self) -> &'static str {
        self.pick("↑↓", "up/dn")
    }

    /// Commits this branch has that its upstream does not.
    pub(super) fn ahead(self) -> &'static str {
        self.pick("↑", "+")
    }

    /// Commits the upstream has that this branch does not.
    pub(super) fn behind(self) -> &'static str {
        self.pick("↓", "-")
    }

    /// Separator between hints on a single status row.
    pub(super) fn separator(self) -> &'static str {
        self.pick("·", "-")
    }

    /// Truncation marker. Single column in both sets so the width arithmetic
    /// that reserves room for it does not have to branch.
    pub(super) fn ellipsis(self) -> &'static str {
        self.pick("…", "~")
    }

    /// The one-column marker for a public agent status.
    pub(super) fn status(self, status: DisplayStatus) -> &'static str {
        match status {
            DisplayStatus::Idle | DisplayStatus::Ready => self.idle(),
            DisplayStatus::Working => self.running(),
            DisplayStatus::NeedsInput => self.attention(),
            DisplayStatus::Done => self.done(),
            DisplayStatus::Exited => self.exited(),
            DisplayStatus::Unknown => self.unsure(),
        }
    }

    /// The one-column marker for a host's connection state.
    pub(super) fn connection(self, connection: HostConnection) -> &'static str {
        match connection {
            HostConnection::Local | HostConnection::Connected => self.connected(),
            HostConnection::Authenticating
            | HostConnection::Reconnecting
            | HostConnection::NeedsAuthentication => self.transitional(),
            HostConnection::HostKeyBlocked | HostConnection::Failed => self.attention(),
            HostConnection::Disconnected => self.disconnected(),
        }
    }
}
