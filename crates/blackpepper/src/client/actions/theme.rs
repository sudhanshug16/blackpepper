//! `:theme` — list the palettes, or switch to one.
//!
//! Switching repaints immediately and is written back to the user config, so
//! the choice survives a restart. The write is a targeted line edit rather
//! than a serialize-and-replace: the config is a file the user owns and may
//! have commented, and a theme change is not a reason to reformat it.

use super::super::ClientState;
use crate::client_config::theme;

pub(super) fn apply(state: &mut ClientState, name: Option<String>) -> Result<(), String> {
    let Some(name) = name else {
        return list(state);
    };
    let Some(theme) = theme::by_name(&name) else {
        return Err(format!(
            "Unknown theme {name:?}. Known themes: {}.",
            theme::names().collect::<Vec<_>>().join(", ")
        ));
    };

    // A user who pinned their own surfaces keeps them; the theme only moves
    // what it still owns.
    let pinned = state.config.ui.background != state.config.ui.theme.canvas
        || state.config.ui.foreground != state.config.ui.theme.ink;
    state.config.ui.theme = theme;
    if !pinned {
        state.config.ui.background = theme.canvas;
        state.config.ui.foreground = theme.ink;
    }

    let saved = match persist(theme.name) {
        Ok(path) => format!("Saved to {}.", path.display()),
        Err(error) => format!("This session only; {error}."),
    };
    // Pinned surfaces are worth saying out loud: the accent will have changed
    // while the background did not, which otherwise looks like a half-applied
    // theme.
    let pinned = if pinned {
        " ui.background/ui.foreground are pinned in config, so surfaces did not change."
    } else {
        ""
    };
    state.set_output(format!(
        "Theme is {} — {}. {saved}{pinned}",
        theme.name, theme.summary
    ));
    Ok(())
}

fn list(state: &mut ClientState) -> Result<(), String> {
    let current = state.config.ui.theme.name;
    let width = theme::names().map(str::len).max().unwrap_or(0);
    let body = theme::THEMES
        .iter()
        .map(|theme| {
            let marker = if theme.name == current { "*" } else { " " };
            format!(
                "{marker} {:width$}  {}",
                theme.name,
                theme.summary,
                width = width
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    state.set_detail(
        "Themes",
        format!("{body}\n\n:theme <name> switches and saves the choice."),
    );
    state.set_output("Theme list open. Use :theme <name> to switch; Esc closes it.");
    Ok(())
}

/// Write `theme = "<name>"` into the `[ui]` table of the user config, adding
/// the key or the table if either is missing and leaving every other byte —
/// comments included — exactly as it was.
fn persist(name: &str) -> Result<std::path::PathBuf, String> {
    let path = crate::client_config::user_config_path()
        .ok_or_else(|| "no user config directory is available".to_owned())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let existing = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let updated = with_theme(&existing, name);
    std::fs::write(&path, updated)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(path)
}

/// Pure text edit, so the rewrite rules are testable without touching disk.
fn with_theme(existing: &str, name: &str) -> String {
    let line = format!("theme = \"{name}\"");
    let mut lines = existing.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut in_ui = false;
    let mut ui_start = None;
    for index in 0..lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with('[') {
            in_ui = trimmed == "[ui]";
            if in_ui {
                ui_start = Some(index);
            }
            continue;
        }
        if in_ui
            && trimmed
                .split('=')
                .next()
                .is_some_and(|key| key.trim() == "theme")
        {
            lines[index] = line;
            return rejoin(lines, existing);
        }
    }
    match ui_start {
        // Insert directly under the header so the key is where a reader looks.
        Some(index) => lines.insert(index + 1, line),
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|last| last.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[ui]".to_owned());
            lines.push(line);
        }
    }
    rejoin(lines, existing)
}

fn rejoin(lines: Vec<String>, existing: &str) -> String {
    let mut joined = lines.join("\n");
    if existing.is_empty() || existing.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::with_theme;

    #[test]
    fn an_empty_config_gains_the_table_and_the_key() {
        assert_eq!(with_theme("", "violet"), "[ui]\ntheme = \"violet\"\n");
    }

    #[test]
    fn an_existing_key_is_replaced_in_place() {
        let existing = "[ui]\n# my note\ntheme = \"brass\"\nglyphs = \"ascii\"\n";
        assert_eq!(
            with_theme(existing, "pink"),
            "[ui]\n# my note\ntheme = \"pink\"\nglyphs = \"ascii\"\n"
        );
    }

    #[test]
    fn an_existing_ui_table_gains_the_key_without_disturbing_others() {
        let existing = "[hosts.lab]\ndestination = \"lab\"\n\n[ui]\nglyphs = \"ascii\"\n";
        assert_eq!(
            with_theme(existing, "none"),
            "[hosts.lab]\ndestination = \"lab\"\n\n[ui]\ntheme = \"none\"\nglyphs = \"ascii\"\n"
        );
    }

    #[test]
    fn a_config_without_a_ui_table_keeps_its_comments() {
        let existing = "# keep me\n[hosts.lab]\ndestination = \"lab\"\n";
        assert_eq!(
            with_theme(existing, "indigo"),
            "# keep me\n[hosts.lab]\ndestination = \"lab\"\n\n[ui]\ntheme = \"indigo\"\n"
        );
    }

    /// A `theme` key belonging to some other table must not be mistaken for
    /// the one under `[ui]`.
    #[test]
    fn a_key_in_another_table_is_left_alone() {
        let existing = "[workspace.env]\ntheme = \"unrelated\"\n";
        assert_eq!(
            with_theme(existing, "violet"),
            "[workspace.env]\ntheme = \"unrelated\"\n\n[ui]\ntheme = \"violet\"\n"
        );
    }
}
