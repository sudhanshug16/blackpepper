//! Merging Blackpepper's Zellij appearance into whatever configuration the
//! host already has.
//!
//! Zellij writes a configuration file into the user's config directory on
//! first run, and rewrites it on upgrade. "The host has no Zellij config" is
//! therefore almost never true, so treating a present file as a reason to skip
//! the appearance means the appearance never applies. What that file usually
//! contains is keybindings — not a single opinion about colour or frames.
//!
//! So the rule is per setting, not per file. Every top-level node Blackpepper
//! would contribute is added only when the host's configuration does not
//! already define that node. A user who sets their own `theme` keeps it; a user
//! who has only ever set keybindings gets Blackpepper's appearance and keeps
//! every binding.
//!
//! The merge is textual because it has to be: Zellij accepts one `--config`
//! file, KDL round-tripping would reformat a file the user owns, and the
//! result is written to a Blackpepper-owned path rather than over the original.

/// The appearance Blackpepper contributes when nothing else claims it.
pub const APPEARANCE: &str = include_str!("../../assets/zellij/config.kdl");

/// Merge `appearance` into `existing`, adding only the top-level nodes that
/// `existing` does not already define. Returns the file to hand to `--config`.
pub fn merge(existing: &str, appearance: &str) -> String {
    let defined = top_level_names(existing);
    let contributed = top_level_blocks(appearance)
        .into_iter()
        .filter(|(name, _)| !defined.contains(name))
        .map(|(_, block)| block)
        .collect::<Vec<_>>();
    if contributed.is_empty() {
        return existing.to_owned();
    }
    let mut merged = existing.trim_end().to_owned();
    if !merged.is_empty() {
        merged.push_str("\n\n");
    }
    merged.push_str(
        "// Added by Blackpepper. Every node below was absent from this host's\n\
         // Zellij configuration; anything it did define has been left alone.\n",
    );
    merged.push_str(&contributed.join("\n"));
    if !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged
}

/// Names of the top-level nodes a configuration defines. Only column-zero
/// lines count, so a `theme` key nested inside some other block is not
/// mistaken for the top-level one.
fn top_level_names(config: &str) -> Vec<String> {
    top_level_blocks(config)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Every top-level node, as `(name, source text)`. Brace depth is tracked so a
/// multi-line node such as `themes { … }` is returned whole.
fn top_level_blocks(config: &str) -> Vec<(String, String)> {
    let mut blocks = Vec::new();
    let mut lines = config.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        // Column-zero, non-blank, non-comment lines start a top-level node.
        if line.starts_with([' ', '\t']) || trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let Some(name) = node_name(trimmed) else {
            continue;
        };
        let mut source = String::from(line);
        let mut depth = brace_delta(line);
        while depth > 0 {
            let Some(next) = lines.next() else { break };
            source.push('\n');
            source.push_str(next);
            depth += brace_delta(next);
        }
        source.push('\n');
        blocks.push((name, source));
    }
    blocks
}

fn node_name(trimmed: &str) -> Option<String> {
    let name = trimmed
        .split([' ', '\t', '{', '='])
        .next()?
        .trim_matches('"')
        .trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Net brace depth of a line, ignoring braces inside a `//` comment. KDL
/// strings in these files never contain braces, so quotes need no special
/// handling.
fn brace_delta(line: &str) -> i32 {
    let code = line.split("//").next().unwrap_or(line);
    code.chars().fold(0, |depth, character| match character {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

#[cfg(test)]
mod tests {
    use super::{merge, top_level_names, APPEARANCE};

    #[test]
    fn the_shipped_appearance_contributes_the_nodes_we_expect() {
        let names = top_level_names(APPEARANCE);
        for expected in ["pane_frames", "simplified_ui", "theme", "themes"] {
            assert!(
                names.iter().any(|name| name == expected),
                "appearance defines no {expected}; found {names:?}"
            );
        }
    }

    /// The case that made this necessary: Zellij autogenerates a keybindings
    /// file, which must not cost the user the whole appearance.
    #[test]
    fn a_keybindings_only_config_keeps_its_bindings_and_gains_the_appearance() {
        let existing = "keybinds clear-defaults=true {\n    pane {\n        bind \"n\" { NewPane; }\n    }\n}\n";
        let merged = merge(existing, APPEARANCE);
        assert!(merged.contains("bind \"n\" { NewPane; }"), "bindings lost");
        assert!(merged.contains("pane_frames false"), "appearance not added");
        assert!(merged.contains("ribbon_selected"), "theme not added");
        assert!(merged.contains("keybinds clear-defaults=true"));
    }

    /// An opinion the user actually expressed outranks ours, per setting.
    #[test]
    fn settings_the_host_already_defines_are_left_alone() {
        let existing = "pane_frames true\ntheme \"catppuccin\"\n";
        let merged = merge(existing, APPEARANCE);
        assert!(merged.contains("pane_frames true"));
        assert!(!merged.contains("pane_frames false"), "overrode the user");
        assert!(merged.contains("theme \"catppuccin\""));
        assert!(
            !merged.contains("theme \"blackpepper\""),
            "overrode the user's theme choice"
        );
        // A setting they did not express is still contributed.
        assert!(merged.contains("simplified_ui"));
    }

    #[test]
    fn an_empty_config_takes_the_whole_appearance() {
        let merged = merge("", APPEARANCE);
        assert!(merged.contains("pane_frames false"));
        assert!(merged.contains("theme \"blackpepper\""));
        assert!(merged.contains("ribbon_unselected"));
    }

    /// A nested key must not be mistaken for a top-level one, or a `theme`
    /// inside a layout block would suppress ours.
    #[test]
    fn nested_keys_do_not_count_as_definitions() {
        let existing = "layout {\n    pane {\n        theme \"other\"\n    }\n}\n";
        let merged = merge(existing, APPEARANCE);
        assert!(
            merged.contains("theme \"blackpepper\""),
            "a nested theme suppressed the top-level one"
        );
    }

    #[test]
    fn a_multi_line_node_is_carried_whole() {
        let existing = "themes {\n    mine {\n        fg 1 2 3\n    }\n}\n";
        let merged = merge(existing, APPEARANCE);
        assert!(merged.contains("mine {"), "existing theme block lost");
        assert!(
            !merged.contains("blackpepper {"),
            "appended a themes block beside the host's own"
        );
    }

    #[test]
    fn merging_twice_changes_nothing_the_second_time() {
        let once = merge("keybinds {\n}\n", APPEARANCE);
        let twice = merge(&once, APPEARANCE);
        assert_eq!(once, twice, "merge is not idempotent");
    }

    #[test]
    fn comments_and_blank_lines_are_not_nodes() {
        let existing = "// a comment\n\n// another\n";
        let merged = merge(existing, APPEARANCE);
        assert!(merged.contains("// a comment"));
        assert!(merged.contains("pane_frames false"));
    }
}
