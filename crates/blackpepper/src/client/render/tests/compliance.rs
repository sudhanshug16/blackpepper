//! Design-compliance checks for the surfaces the v2 spec names explicitly.

use super::{buffer_text, draw, workspace_state};
use crate::client::{ClientState, DisplayStatus, HostConnection};
use crate::client_config::GlyphSet;
use crate::core::{PullRequestState, PullRequestSummary, WorkspaceOverview};

/// Every glyph the design budgets, and nothing else, may reach the screen.
const BUDGET: [&str; 12] = ["●", "○", "◐", "▸", "!", "✓", "×", "?", "⚠", "·", "→", "…"];

/// Half-blocks for the mark, box-drawing-free spinner phases, and the two
/// divergence arrows are the remaining sanctioned non-ASCII characters.
const ALSO_ALLOWED: [&str; 17] = [
    "█", "▀", "▄", "↑", "↓", "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " ", "\n",
];

fn running_state() -> ClientState {
    let mut state = workspace_state();
    let workspace_id = state.selected_workspace.unwrap();
    let run = crate::client::state::AgentRunView {
        run_id: crate::core::AgentRunId::new(),
        pane_id: crate::core::PaneId::new(),
        tab_id: 1,
        provider: crate::agent_status::Provider::Codex,
        zellij_pane_id: "1".to_owned(),
        needs_input_capability: "exact".to_owned(),
        snapshot: Some(crate::agent_status::AgentSnapshot {
            run_id: crate::core::AgentRunId::new(),
            provider: crate::agent_status::Provider::Codex,
            state: crate::agent_status::AgentState::Working,
            revision: 1,
            completion_revision: 0,
            seen_completion_revision: 0,
            last_event_sequence: Some(1),
            last_event_at_ms: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64
                    - 134_000,
            ),
            integration_health: crate::agent_status::IntegrationHealth::Healthy {
                integration_version: Some(1),
            },
            needs_input_capability: crate::agent_status::NeedsInputCapability::ProviderEvents,
            completion_suppressed: false,
        }),
        explain: None,
        snapshot_error: None,
        seen_completion_revision: 0,
        blocker: None,
        blocker_watcher_instance: None,
        blocker_sequence: 0,
        blocker_observed_at_ms: None,
        interrupted_after_sequence: None,
    };
    state.agent_runs.insert(workspace_id, vec![run]);
    state.refresh_workspace_status(workspace_id);
    state.rebuild_tree();
    state
}

#[test]
fn a_running_agent_shows_its_provider_and_the_age_of_its_last_event() {
    let mut state = running_state();
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    // 134 seconds ago reads as "2m", not as the vocabulary word.
    assert!(
        rendered.contains("▸ 2m"),
        "missing elapsed status in:\n{rendered}"
    );
    assert!(!rendered.contains("▸ running"));
}

#[test]
fn the_host_row_reports_reachability_rather_than_agent_state() {
    let mut state = running_state();
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    assert!(
        rendered.contains("local"),
        "missing host state in:\n{rendered}"
    );
    for connection in [
        HostConnection::Authenticating,
        HostConnection::Disconnected,
        HostConnection::Failed,
    ] {
        assert!(!connection.public_word().is_empty());
    }
}

#[test]
fn the_ascii_flag_replaces_every_budgeted_glyph() {
    let mut state = running_state();
    state.config.ui.glyphs = GlyphSet::Ascii;
    state.host_operations.insert(
        state.snapshot.hosts[0].id,
        (uuid::Uuid::new_v4(), "connecting".to_owned()),
    );
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    // `!` and `?` are already ASCII and legitimately survive the switch; the
    // check is that nothing outside ASCII does.
    assert!(
        rendered.is_ascii(),
        "ASCII mode still emitted {:?} in:\n{rendered}",
        rendered
            .chars()
            .filter(|character| !character.is_ascii())
            .collect::<String>()
    );
    // The status column keeps its meaning, only its marker changes.
    assert!(
        rendered.contains("> 2m"),
        "missing ASCII status:\n{rendered}"
    );
}

#[test]
fn unicode_mode_stays_inside_the_glyph_budget() {
    let mut state = running_state();
    state.host_operations.insert(
        state.snapshot.hosts[0].id,
        (uuid::Uuid::new_v4(), "connecting".to_owned()),
    );
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    for character in rendered.chars().filter(|character| !character.is_ascii()) {
        let glyph = character.to_string();
        assert!(
            BUDGET.contains(&glyph.as_str()) || ALSO_ALLOWED.contains(&glyph.as_str()),
            "unbudgeted glyph {glyph:?} reached the screen in:\n{rendered}"
        );
    }
}

#[test]
fn the_header_carries_branch_divergence_and_pull_request() {
    let mut state = workspace_state();
    let workspace_id = state.selected_workspace.unwrap();
    state.overviews.insert(
        workspace_id,
        WorkspaceOverview {
            head: Some("main".to_owned()),
            dirty: true,
            ahead: 2,
            behind: 0,
            pull_request: Some(PullRequestSummary {
                number: 418,
                state: PullRequestState::Open,
            }),
            active_tab: Some(2),
            tab_count: Some(4),
        },
    );
    let terminal = draw(&mut state, 120, 24);
    let header = super::row_text(&terminal, 0);
    assert!(
        header.contains("main*"),
        "missing dirty branch in: {header}"
    );
    assert!(header.contains("↑2"), "missing divergence in: {header}");
    assert!(
        header.contains("PR #418 open"),
        "missing pull request in: {header}"
    );

    let rendered = buffer_text(&terminal);
    assert!(
        rendered.contains("tab 2/4"),
        "missing tab position in:\n{rendered}"
    );
}

#[test]
fn an_unreported_workspace_shows_no_branch_rather_than_a_stale_one() {
    let mut state = workspace_state();
    let terminal = draw(&mut state, 120, 24);
    let header = super::row_text(&terminal, 0);
    assert!(!header.contains("main"), "invented a branch in: {header}");
    assert!(header.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
}

#[test]
fn help_groups_by_target_and_dims_what_cannot_run_with_the_reason() {
    let mut state = workspace_state();
    state.help = Some(crate::client::state::HelpView::default());
    let rendered = buffer_text(&draw(&mut state, 110, 40));
    for heading in ["THIS WORKSPACE", "REPOSITORY", "HOSTS"] {
        assert!(
            rendered.contains(heading),
            "missing group {heading} in:\n{rendered}"
        );
    }
    // Nothing is under review, so :approve is listed with its reason.
    assert!(
        rendered.contains(":approve") && rendered.contains("nothing under review"),
        "unavailable command lost its reason in:\n{rendered}"
    );
    assert!(
        rendered.contains(":agent spawn <provider>") && rendered.contains("codex · claude"),
        "missing grounded provider list in:\n{rendered}"
    );
}

#[test]
fn completion_offers_only_listeners_this_client_has_discovered() {
    let mut state = workspace_state();
    let workspace = state.snapshot.workspaces[0].clone();
    state.ports.insert(
        workspace.host_id,
        crate::ports::PortSnapshot {
            listeners: vec![crate::ports::PortListener {
                bind_address: "127.0.0.1".to_owned(),
                port: 3000,
                pid: Some(1),
                process: Some("node".to_owned()),
                workspace_path: Some(workspace.root_path.clone().into()),
                attribution: crate::ports::AttributionConfidence::ExactCwd,
            }],
            completeness: crate::ports::ProbeCompleteness::Full,
            warning: None,
        },
    );
    state.command_active = true;
    state.command_input = ":forward ".to_owned();

    let rendered = buffer_text(&draw(&mut state, 110, 24));
    assert!(
        rendered.contains("forward 3000") && rendered.contains("discovered · node"),
        "missing grounded candidate in:\n{rendered}"
    );
    // A port nobody is listening on is never offered.
    assert!(!rendered.contains("forward 5432"));

    let candidates = crate::client::completion::candidates(&state, "forward ");
    let listener_values = candidates
        .iter()
        .filter(|candidate| candidate.value != "forward cancel")
        .map(|candidate| candidate.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(listener_values, ["forward 3000"]);
    assert!(candidates
        .iter()
        .any(|candidate| candidate.value == "forward cancel"));
}

#[test]
fn completion_uses_exact_addresses_when_a_port_has_multiple_listeners() {
    let mut state = workspace_state();
    let workspace = state.snapshot.workspaces[0].clone();
    state.ports.insert(
        workspace.host_id,
        crate::ports::PortSnapshot {
            listeners: ["127.0.0.1", "127.0.0.2"]
                .into_iter()
                .map(|address| crate::ports::PortListener {
                    bind_address: address.to_owned(),
                    port: 3000,
                    pid: Some(1),
                    process: Some("node".to_owned()),
                    workspace_path: Some(workspace.root_path.clone().into()),
                    attribution: crate::ports::AttributionConfidence::ExactCwd,
                })
                .collect(),
            completeness: crate::ports::ProbeCompleteness::Full,
            warning: None,
        },
    );

    let values = crate::client::completion::candidates(&state, "forward ")
        .into_iter()
        .map(|candidate| candidate.value)
        .collect::<Vec<_>>();
    assert!(values.contains(&"forward 127.0.0.1:3000".to_owned()));
    assert!(values.contains(&"forward 127.0.0.2:3000".to_owned()));
    assert!(!values.contains(&"forward 3000".to_owned()));
}

#[test]
fn the_command_bar_shows_the_argument_it_is_waiting_for() {
    let mut state = workspace_state();
    state.command_active = true;
    state.command_input = ":forward ".to_owned();
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    assert!(
        rendered.contains("<port|address:port>"),
        "missing argument placeholder in:\n{rendered}"
    );
}

#[test]
fn argument_completion_quotes_workspace_names_and_round_trips_through_the_parser() {
    let mut state = workspace_state();
    state.snapshot.workspaces[0].display_name = Some("black pepper".to_owned());
    state.rebuild_tree();

    let candidates = crate::client::completion::candidates(&state, "workspace switch black");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].value, "workspace switch 'black pepper'");
    assert_eq!(
        crate::client::parse_command(&format!(":{}", candidates[0].value)).unwrap(),
        crate::client::ClientCommand::WorkspaceSwitch {
            selector: "black pepper".to_owned(),
        }
    );
}

#[test]
fn the_picker_filters_across_hosts_and_keeps_the_status_column() {
    let mut state = running_state();
    state.open_picker();
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    assert!(rendered.contains("SWITCH TO"));
    assert!(rendered.contains("blackpepper"));
    assert!(rendered.contains("type to filter"));
    assert!(
        rendered.contains("▸ 2m"),
        "picker dropped the status column in:\n{rendered}"
    );

    // A filter that matches nothing says so rather than showing everything.
    state.picker.as_mut().unwrap().filter = "zzz".to_owned();
    assert!(state.picker_matches().is_empty());
    assert!(state.picker_choice().is_none());
    let rendered = buffer_text(&draw(&mut state, 110, 24));
    assert!(rendered.contains("no workspace matches that filter"));
}

#[test]
fn the_picker_cursor_stays_inside_the_filtered_list() {
    let mut state = running_state();
    state.open_picker();
    state.move_picker(5);
    assert_eq!(state.picker.as_ref().unwrap().selected, 0);
    state.move_picker(-5);
    assert_eq!(state.picker.as_ref().unwrap().selected, 0);
    assert_eq!(state.picker_choice(), state.selected_workspace);
}

#[test]
fn both_status_rows_anchor_left_and_put_their_hints_hard_right() {
    let mut state = running_state();
    state.mode = crate::client::ClientMode::Work;
    let terminal = draw(&mut state, 100, 24);
    let work = super::row_text(&terminal, 23);
    assert!(work.starts_with("  bp  blackpepper"), "work row: {work:?}");
    assert!(
        work.trim_end().ends_with("^\\ list"),
        "work hints not right-aligned: {work:?}"
    );

    state.mode = crate::client::ClientMode::Manage;
    let terminal = draw(&mut state, 100, 24);
    let manage = super::row_text(&terminal, 23);
    assert!(
        manage.starts_with("   MANAGE "),
        "manage row lost its badge: {manage:?}"
    );
}

#[test]
fn the_manage_row_names_what_is_asking_on_its_right_edge() {
    let mut state = running_state();
    let workspace_id = state.selected_workspace.unwrap();
    state
        .statuses
        .insert(workspace_id, DisplayStatus::NeedsInput);
    state.rebuild_tree();

    let terminal = draw(&mut state, 100, 24);
    let manage = super::row_text(&terminal, 23);
    assert!(
        manage.trim_end().ends_with("! blackpepper asks"),
        "attention not right-aligned: {manage:?}"
    );
}

#[test]
fn every_public_status_keeps_one_glyph_and_one_word() {
    let state = workspace_state();
    let cases = [
        (DisplayStatus::Idle, "· idle"),
        (DisplayStatus::Ready, "· idle"),
        (DisplayStatus::Working, "▸ running"),
        (DisplayStatus::NeedsInput, "! asks"),
        (DisplayStatus::Done, "✓ done"),
        (DisplayStatus::Exited, "× exited"),
        (DisplayStatus::Unknown, "? unsure"),
    ];
    for (status, expected) in cases {
        assert_eq!(
            super::super::style::status_text(&state, status, None),
            expected
        );
    }
}

/// At full colour depth the client names every colour exactly. A named ANSI
/// slot here would be filled in by the user's terminal theme, which is how the
/// palette drifted from the design the first time.
#[test]
fn full_colour_depth_never_defers_to_a_terminal_slot() {
    use ratatui::style::Color;
    let mut state = running_state();
    state
        .statuses
        .insert(state.selected_workspace.unwrap(), DisplayStatus::NeedsInput);
    state.host_operations.insert(
        state.snapshot.hosts[0].id,
        (uuid::Uuid::new_v4(), "connecting".to_owned()),
    );
    state.rebuild_tree();
    let terminal = draw(&mut state, 120, 24);
    let buffer = terminal.backend().buffer();
    for row in 0..buffer.area.height {
        for column in 0..buffer.area.width {
            let cell = buffer.cell((column, row)).unwrap();
            for (role, colour) in [("fg", cell.fg), ("bg", cell.bg)] {
                assert!(
                    matches!(colour, Color::Rgb(..) | Color::Reset),
                    "cell ({column},{row}) {role} is {colour:?}, a slot the terminal theme fills in"
                );
            }
        }
    }
}

/// The design's palette, cell for cell.
#[test]
fn the_palette_is_the_designs_palette() {
    use ratatui::style::Color;
    let state = workspace_state();
    use super::super::style;
    assert_eq!(
        style::ui_style(&state).bg,
        Some(Color::Rgb(0x1c, 0x1d, 0x1f))
    );
    assert_eq!(
        style::ui_style(&state).fg,
        Some(Color::Rgb(0xe6, 0xe4, 0xe1))
    );
    assert_eq!(
        style::panel_style(&state).bg,
        Some(Color::Rgb(0x23, 0x24, 0x27))
    );
    assert_eq!(
        style::accent_style(&state).fg,
        Some(Color::Rgb(0xb8, 0xa0, 0x4a))
    );
    assert_eq!(
        style::mid_style(&state).fg,
        Some(Color::Rgb(0xb9, 0xb6, 0xb2))
    );
    assert_eq!(
        style::section_style(&state).fg,
        Some(Color::Rgb(0x6c, 0x6b, 0x68))
    );
    assert_eq!(
        style::warning_style(&state).fg,
        Some(Color::Rgb(0xe5, 0xc0, 0x7b))
    );
    assert_eq!(
        style::danger_style(&state).fg,
        Some(Color::Rgb(0xe0, 0x6c, 0x75))
    );
}

/// Every palette must paint a complete, exact frame. A theme that renders a
/// terminal slot somewhere would inherit the user's scheme in that one spot.
#[test]
fn every_theme_paints_an_exact_frame() {
    use ratatui::style::Color;
    for theme in crate::client_config::theme::THEMES {
        let mut state = running_state();
        state.config.ui.theme = theme;
        state.config.ui.background = theme.canvas;
        state.config.ui.foreground = theme.ink;
        let terminal = draw(&mut state, 120, 24);
        let buffer = terminal.backend().buffer();
        for row in 0..buffer.area.height {
            for column in 0..buffer.area.width {
                let cell = buffer.cell((column, row)).unwrap();
                for colour in [cell.fg, cell.bg] {
                    assert!(
                        matches!(colour, Color::Rgb(..) | Color::Reset),
                        "theme {} left {colour:?} at ({column},{row})",
                        theme.name
                    );
                }
            }
        }
        // The canvas is the theme's own, so a light theme really is light.
        assert_eq!(
            buffer.cell((60, 12)).unwrap().bg,
            Color::Rgb(theme.canvas.0, theme.canvas.1, theme.canvas.2),
            "theme {} did not paint its canvas",
            theme.name
        );
    }
}

/// At the sixteen-colour floor no theme may paint its brand in a colour the
/// status vocabulary already owns — that is what makes an alert unmissable.
#[test]
fn no_theme_confuses_its_brand_with_an_alert_at_the_floor() {
    use crate::client_config::ColorTier;
    let mut state = running_state();
    state.config.ui.color_tier = ColorTier::Ansi16;
    for theme in crate::client_config::theme::THEMES {
        state.config.ui.theme = theme;
        let accent = super::super::style::accent_style(&state).fg;
        for status in [
            DisplayStatus::Working,
            DisplayStatus::NeedsInput,
            DisplayStatus::Done,
            DisplayStatus::Exited,
        ] {
            let status_colour = super::super::style::status_style(&state, status).fg;
            assert!(
                accent.is_none() || accent != status_colour,
                "theme {} paints its accent the same as {status:?} at sixteen colours",
                theme.name
            );
        }
    }
}

/// The command bar must not resize the body it sits over. Taking rows from
/// the layout reflows the attached session — and its PTY — on every keystroke.
#[test]
fn the_completion_list_overlays_rather_than_resizing_the_session() {
    let mut state = running_state();
    let workspace = state.snapshot.workspaces[0].clone();
    state.ports.insert(
        workspace.host_id,
        crate::ports::PortSnapshot {
            listeners: vec![crate::ports::PortListener {
                bind_address: "127.0.0.1".to_owned(),
                port: 3000,
                pid: Some(1),
                process: Some("node".to_owned()),
                workspace_path: Some(workspace.root_path.clone().into()),
                attribution: crate::ports::AttributionConfidence::ExactCwd,
            }],
            completeness: crate::ports::ProbeCompleteness::Full,
            warning: None,
        },
    );

    draw(&mut state, 120, 24);
    let closed = state.terminal_area.expect("session area while closed");

    // Open the bar and type enough to produce a list.
    state.command_active = true;
    state.command_input = ":forward ".to_owned();
    let terminal = draw(&mut state, 120, 24);
    let open = state.terminal_area.expect("session area while open");

    assert_eq!(
        closed, open,
        "the session was resized when the command bar opened"
    );
    // And the list really is on screen, over the body.
    let rendered = buffer_text(&terminal);
    assert!(
        rendered.contains("forward 3000"),
        "completion list did not draw over the body"
    );
}
