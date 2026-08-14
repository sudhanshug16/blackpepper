use std::collections::BTreeMap;

const TERM_PROGRAM: &str = "TERM_PROGRAM";
const TERM_PROGRAM_VERSION: &str = "TERM_PROGRAM_VERSION";

/// Fill identity variables that are absent from the configured environment.
///
/// Zellij normalizes `TERM`, but programs in its panes still need to know the
/// capabilities of the real terminal attached to Blackpepper.
///
/// Project configuration keeps precedence when it explicitly supplies these
/// variables. The caller applies them only to the new agent command; the
/// persistent Zellij session and its existing panes are never mutated.
pub(super) fn apply(environment: &mut BTreeMap<String, String>) {
    apply_with(environment, |key| std::env::var(key).ok());
}

pub(super) fn apply_with(
    environment: &mut BTreeMap<String, String>,
    mut read: impl FnMut(&str) -> Option<String>,
) {
    // Treat the configured pair atomically. Filling half of an explicitly
    // configured identity from the outer client could create combinations
    // such as `WezTerm` with Ghostty's version.
    if environment.contains_key(TERM_PROGRAM) || environment.contains_key(TERM_PROGRAM_VERSION) {
        return;
    }
    let terminal_program = non_empty(read(TERM_PROGRAM)).or_else(|| {
        (read("TERM").as_deref() == Some("xterm-ghostty")).then(|| "ghostty".to_owned())
    });
    if let Some(value) = terminal_program {
        environment.insert(TERM_PROGRAM.to_owned(), value);
        if let Some(version) = non_empty(read(TERM_PROGRAM_VERSION)) {
            environment.insert(TERM_PROGRAM_VERSION.to_owned(), version);
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_from(environment: &mut BTreeMap<String, String>, source: BTreeMap<&str, &str>) {
        apply_with(environment, |key| source.get(key).map(ToString::to_string));
    }

    #[test]
    fn preserves_explicit_terminal_program_and_version() {
        let source = BTreeMap::from([
            ("TERM", "xterm-ghostty"),
            (TERM_PROGRAM, "WezTerm"),
            (TERM_PROGRAM_VERSION, "20240203-110809"),
        ]);
        let mut environment = BTreeMap::new();

        apply_from(&mut environment, source);

        assert_eq!(environment.get(TERM_PROGRAM).unwrap(), "WezTerm");
        assert_eq!(
            environment.get(TERM_PROGRAM_VERSION).unwrap(),
            "20240203-110809"
        );
    }

    #[test]
    fn infers_ghostty_only_from_its_exact_term_name() {
        let mut ghostty = BTreeMap::new();
        apply_from(&mut ghostty, BTreeMap::from([("TERM", "xterm-ghostty")]));
        assert_eq!(ghostty.get(TERM_PROGRAM).unwrap(), "ghostty");

        let mut unknown = BTreeMap::new();
        apply_from(
            &mut unknown,
            BTreeMap::from([("TERM", "xterm-ghostty-direct")]),
        );
        assert!(!unknown.contains_key(TERM_PROGRAM));
    }

    #[test]
    fn explicit_workspace_values_keep_normal_config_precedence() {
        let mut environment = BTreeMap::from([
            ("UNRELATED".to_owned(), "kept".to_owned()),
            (TERM_PROGRAM.to_owned(), "project-value".to_owned()),
            (
                TERM_PROGRAM_VERSION.to_owned(),
                "project-version".to_owned(),
            ),
        ]);
        let source = BTreeMap::from([(TERM_PROGRAM, "ghostty"), (TERM_PROGRAM_VERSION, "1.3.1")]);

        apply_from(&mut environment, source);

        assert_eq!(environment.get(TERM_PROGRAM).unwrap(), "project-value");
        assert_eq!(
            environment.get(TERM_PROGRAM_VERSION).unwrap(),
            "project-version"
        );
        assert_eq!(environment.get("UNRELATED").unwrap(), "kept");
    }

    #[test]
    fn a_partial_configured_identity_is_never_paired_with_the_outer_terminal() {
        let source = BTreeMap::from([(TERM_PROGRAM, "ghostty"), (TERM_PROGRAM_VERSION, "1.3.1")]);
        let mut configured_program =
            BTreeMap::from([(TERM_PROGRAM.to_owned(), "WezTerm".to_owned())]);
        let mut configured_version = BTreeMap::from([(
            TERM_PROGRAM_VERSION.to_owned(),
            "project-version".to_owned(),
        )]);

        apply_from(&mut configured_program, source.clone());
        apply_from(&mut configured_version, source);

        assert_eq!(configured_program.get(TERM_PROGRAM).unwrap(), "WezTerm");
        assert!(!configured_program.contains_key(TERM_PROGRAM_VERSION));
        assert!(!configured_version.contains_key(TERM_PROGRAM));
        assert_eq!(
            configured_version.get(TERM_PROGRAM_VERSION).unwrap(),
            "project-version"
        );
    }
}
