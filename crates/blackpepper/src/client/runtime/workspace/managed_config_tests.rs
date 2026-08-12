//! Guards on the Zellij appearance Blackpepper installs when a host has none
//! of its own.
//!
//! Zellij 0.44.3's `setup --check` only validates KDL syntax — it accepts
//! unknown keys and malformed colour values without complaint — so the
//! properties that matter are asserted here instead of trusted to the binary.

const CONFIG: &str = include_str!("../../../../assets/zellij/config.kdl");

/// The one setting the dashboard's own borderless rule depends on. A frame
/// around every pane costs two rows and two columns and reinstates exactly the
/// noise the design removes.
#[test]
fn panes_are_edge_to_edge() {
    assert!(
        CONFIG.contains("pane_frames false"),
        "managed config must disable pane frames"
    );
}

/// Chrome colour belongs to the theme's UI components. Recolouring the
/// sixteen-colour palette to style tabs also recolours everything programs
/// print inside a pane — a test suite's green "ok" would stop being green.
#[test]
fn the_ansi_palette_stays_faithful_to_its_own_names() {
    for (name, expected_dominant) in [("green", 1usize), ("red", 0), ("blue", 2)] {
        let channels =
            palette_entry(name).unwrap_or_else(|| panic!("managed config defines no {name}"));
        let dominant = channels
            .iter()
            .enumerate()
            .max_by_key(|(_, value)| **value)
            .map(|(index, _)| index)
            .expect("three channels");
        assert_eq!(
            dominant, expected_dominant,
            "{name} is {channels:?}, which is not {name}; chrome must use the theme's UI \
             components rather than repainting the palette programs render with"
        );
    }
}

/// Tabs are flat rectangles, which in Zellij's vocabulary is a ribbon. Without
/// these the tab bar falls back to Zellij's own styling.
#[test]
fn tabs_are_styled_as_ribbons() {
    for component in ["ribbon_selected", "ribbon_unselected"] {
        assert!(
            CONFIG.contains(component),
            "managed config must style {component}"
        );
    }
}

/// Every component Zellij accepts takes a base, a background, and four
/// emphasis slots; a short block silently loses colours.
#[test]
fn every_styled_component_is_complete() {
    for block in [
        "text_unselected",
        "text_selected",
        "ribbon_unselected",
        "ribbon_selected",
        "list_unselected",
        "list_selected",
        "table_title",
        "table_cell_unselected",
        "table_cell_selected",
        "frame_unselected",
        "frame_selected",
        "frame_highlight",
        "exit_code_success",
        "exit_code_error",
    ] {
        let body =
            component_body(block).unwrap_or_else(|| panic!("managed config defines no {block}"));
        for field in ["base", "background", "emphasis_0", "emphasis_3"] {
            assert!(body.contains(field), "{block} is missing {field}");
        }
        for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let channels = line.split_whitespace().skip(1).count();
            assert_eq!(channels, 3, "{block} has a non-RGB entry: {line:?}");
        }
    }
}

fn palette_entry(name: &str) -> Option<[u16; 3]> {
    let line = CONFIG
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(&format!("{name} ")))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse().ok())
        .collect::<Vec<u16>>();
    <[u16; 3]>::try_from(values.as_slice()).ok()
}

fn component_body(name: &str) -> Option<&'static str> {
    let start = CONFIG.find(&format!("{name} {{"))?;
    let open = CONFIG[start..].find('{')? + start + 1;
    let close = CONFIG[open..].find('}')? + open;
    Some(&CONFIG[open..close])
}
