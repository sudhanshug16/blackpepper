use super::{buffer_text, draw, row_text, workspace_state};
use crate::client::{ClientMode, DisplayStatus};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

#[test]
fn work_mode_gives_zellij_every_cell_except_one_status_row() {
    let mut state = workspace_state();
    state.mode = ClientMode::Work;
    state.ports_area = Some(Rect::new(10, 4, 20, 8));

    let terminal = draw(&mut state, 180, 52);

    assert_eq!(state.terminal_area, Some(Rect::new(0, 0, 180, 51)));
    assert!(state.ports_area.is_none());
    assert!(state.mouse_targets.iter().all(|target| matches!(
        target.action,
        crate::client::state::MouseAction::EnterManage
    )));
    let rendered = buffer_text(&terminal);
    assert!(rendered.contains("bp  blackpepper"));
    assert!(!rendered.contains(" MANAGE "));
    assert!(!rendered.contains(" AUTHENTICATE "));
    assert!(!rendered.contains("HOSTS"));
    assert!(!rendered.contains("PORTS"));
}

#[test]
fn work_footer_at_62_columns_keeps_status_and_manage_for_a_long_workspace_name() {
    let mut state = workspace_state();
    let workspace_id = state.active_workspace.unwrap();
    state.snapshot.workspaces[0].display_name =
        Some("a-very-long-workspace-name-that-must-not-hide-critical-controls".to_owned());
    state
        .statuses
        .insert(workspace_id, DisplayStatus::NeedsInput);
    state.mode = ClientMode::Work;

    let terminal = draw(&mut state, 62, 24);
    let footer = row_text(&terminal, 23);

    assert!(footer.contains("! asks"), "status clipped from {footer:?}");
    assert!(
        footer.contains("^] manage"),
        "Manage chord clipped from {footer:?}"
    );
    assert!(
        footer.contains('…'),
        "workspace was not truncated: {footer:?}"
    );
}

#[test]
fn short_command_palette_registers_only_visible_candidate_rows() {
    let mut state = workspace_state();
    state.command_active = true;
    state.command_input = ":".to_owned();
    state.command_selection = Some(7);

    draw(&mut state, 62, 6);

    let candidates = state
        .mouse_targets
        .iter()
        .filter(|target| {
            matches!(
                target.action,
                crate::client::state::MouseAction::ChooseCompletion(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 3);
    assert!(matches!(
        candidates[0].action,
        crate::client::state::MouseAction::ChooseCompletion(5)
    ));
    assert!(matches!(
        candidates[1].action,
        crate::client::state::MouseAction::ChooseCompletion(6)
    ));
    assert!(matches!(
        candidates[2].action,
        crate::client::state::MouseAction::ChooseCompletion(7)
    ));
    assert!(candidates
        .iter()
        .all(|target| target.area.y.saturating_add(target.area.height) <= 5));
}

#[test]
fn clipped_attention_text_has_no_invisible_footer_action() {
    let mut state = workspace_state();
    let workspace_id = state.selected_workspace.unwrap();
    state
        .statuses
        .insert(workspace_id, DisplayStatus::NeedsInput);
    state.rebuild_tree();

    let terminal = draw(&mut state, 20, 24);
    assert!(!row_text(&terminal, 23).contains("asks"));
    assert!(!state.mouse_targets.iter().any(|target| {
        target.area.y == 23
            && matches!(
                target.action,
                crate::client::state::MouseAction::SelectWorkspace(_)
            )
    }));
}

#[test]
fn exact_wide_threshold_keeps_32_40_30_columns() {
    let mut state = workspace_state();
    draw(&mut state, 102, 24);

    assert_eq!(state.terminal_area, Some(Rect::new(32, 2, 40, 21)));
    assert_eq!(state.ports_area, Some(Rect::new(72, 1, 30, 22)));
}

#[test]
fn fixed_workspace_column_never_clips_the_selected_attention_state() {
    let mut state = workspace_state();
    let workspace_id = state.selected_workspace.unwrap();
    state.connected_clients.insert(workspace_id, 1);
    state
        .statuses
        .insert(workspace_id, DisplayStatus::NeedsInput);
    state.rebuild_tree();

    let terminal = draw(&mut state, 62, 24);
    let buffer = terminal.backend().buffer();
    let workspace_row = (0..buffer.area.height)
        .find(|row| {
            (0..32)
                .filter_map(|column| buffer.cell((column, *row)))
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("●  blackpepper")
        })
        .expect("selected workspace row");
    let row = (0..32)
        .filter_map(|column| buffer.cell((column, workspace_row)))
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(row.contains("! asks"), "status clipped from {row:?}");
    assert!(!row.contains("client"));
    assert!(buffer
        .cell((31, workspace_row))
        .unwrap()
        .modifier
        .contains(Modifier::REVERSED));
}

#[test]
fn fixed_workspace_column_keeps_setup_failure_explicit_without_color() {
    let mut state = workspace_state();
    state.snapshot.workspaces[0].setup = crate::core::WorkspaceSetup::Failed {
        message: "fixture failed".to_owned(),
    };
    state.config.ui.color_tier = crate::client_config::ColorTier::NoColor;
    state.rebuild_tree();

    let rendered = buffer_text(&draw(&mut state, 62, 24));
    assert!(rendered.contains("blackpepper"));
    assert!(rendered.contains("⚠ setup failed"));
}

#[test]
fn overflowing_compact_selector_keeps_selected_setup_failure_visible_without_color() {
    let mut state = workspace_state();
    let selected_id = state.selected_workspace.unwrap();
    let host_id = state.snapshot.workspaces[0].host_id;
    let repository = state.snapshot.workspaces[0].repository.clone();
    state.snapshot.workspaces[0].display_name = Some("zz-selected-failed".to_owned());
    state.snapshot.workspaces[0].setup = crate::core::WorkspaceSetup::Failed {
        message: "fixture failed".to_owned(),
    };
    for index in 0..8 {
        let mut workspace =
            crate::core::WorkspaceRecord::new(host_id, format!("/workspace/overflow-{index}"));
        workspace.display_name = Some(format!("workspace-{index:02}"));
        workspace.repository = repository.clone();
        state.snapshot.workspaces.push(workspace);
    }
    state.config.ui.color_tier = crate::client_config::ColorTier::NoColor;
    state.selected_workspace = Some(selected_id);
    state.set_detail("fixture", "focused view");
    state.rebuild_tree();

    let terminal = draw(&mut state, 61, 24);
    let selector = (1..=4)
        .map(|row| row_text(&terminal, row))
        .collect::<String>();

    assert!(
        selector.contains("zz-selected-failed"),
        "selected row hidden from {selector:?}"
    );
    assert!(
        selector.contains("⚠ setup failed"),
        "setup warning hidden from {selector:?}"
    );
}

#[test]
fn wide_layout_gives_all_extra_width_to_the_session() {
    let mut state = workspace_state();
    draw(&mut state, 140, 30);

    assert_eq!(state.terminal_area, Some(Rect::new(32, 2, 78, 27)));
    assert_eq!(state.ports_area, Some(Rect::new(110, 1, 30, 28)));
}

#[test]
fn every_required_medium_width_preserves_real_terminal_columns() {
    for width in [101, 100, 80, 66, 65, 62] {
        let mut state = workspace_state();
        let terminal = draw(&mut state, width, 24);

        assert_eq!(
            state.terminal_area,
            Some(Rect::new(32, 2, width - 32, 13)),
            "width {width}"
        );
        assert!(state.terminal_area.unwrap().width >= 30, "width {width}");
        assert_eq!(
            state.ports_area,
            Some(Rect::new(32, 15, width - 32, 8)),
            "width {width}"
        );
        assert!(row_text(&terminal, 0).contains("bp  blackpepper"));
        assert!(row_text(&terminal, 23).contains(" MANAGE "));
    }
}

#[test]
fn compact_layout_below_62_keeps_selector_session_and_footer() {
    let mut state = workspace_state();
    let terminal = draw(&mut state, 61, 24);

    assert_eq!(state.terminal_area, Some(Rect::new(0, 8, 61, 15)));
    assert!(state.ports_area.is_none());
    assert!(buffer_text(&terminal).contains("HOSTS"));
    assert!(buffer_text(&terminal).contains("SESSION"));
    assert!(row_text(&terminal, 23).contains("enter attach"));
}

#[test]
fn manage_surfaces_are_borderless_and_use_section_labels() {
    let mut state = workspace_state();
    let terminal = draw(&mut state, 140, 30);
    let rendered = buffer_text(&terminal);

    for old_marker in [
        "Hosts / Workspaces",
        " Zellij ",
        " Ports ",
        "┌",
        "┐",
        "└",
        "┘",
    ] {
        assert!(!rendered.contains(old_marker), "old marker {old_marker:?}");
    }
    for marker in ["HOSTS", "SESSION", "PORTS", " MANAGE "] {
        assert!(rendered.contains(marker), "v2 marker {marker:?}");
    }
}

#[test]
fn transient_output_stays_visible_in_the_single_status_row() {
    let mut state = workspace_state();
    state.set_transient_output(
        "Copy sent to your terminal.",
        std::time::Duration::from_secs(30),
    );
    let terminal = draw(&mut state, 100, 24);

    assert!(row_text(&terminal, 23).contains("Copy sent to your terminal."));
    assert!(row_text(&terminal, 0).contains(&format!("v{}", env!("CARGO_PKG_VERSION"))));
}
