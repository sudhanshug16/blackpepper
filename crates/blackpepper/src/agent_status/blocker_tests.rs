use super::*;

const CODEX_BLOCKED: &str = include_str!("fixtures/codex_blocked.txt");
const CODEX_IDLE: &str = include_str!("fixtures/codex_idle.txt");
const CLAUDE_BLOCKED: &str = include_str!("fixtures/claude_blocked.txt");
const CLAUDE_IDLE: &str = include_str!("fixtures/claude_idle.txt");
const OPENCODE_BLOCKED: &str = include_str!("fixtures/opencode_blocked.txt");
const OPENCODE_IDLE: &str = include_str!("fixtures/opencode_idle.txt");

#[test]
fn bundled_manifests_parse_and_match_realistic_fixtures() {
    for (provider, blocked, idle) in [
        (Provider::Codex, CODEX_BLOCKED, CODEX_IDLE),
        (Provider::Claude, CLAUDE_BLOCKED, CLAUDE_IDLE),
        (Provider::OpenCode, OPENCODE_BLOCKED, OPENCODE_IDLE),
    ] {
        let overlay = BlockerOverlay::bundled(provider).unwrap();
        assert_eq!(overlay.provider(), provider);
        assert!(
            overlay
                .evaluate(BlockerInput {
                    viewport: blocked,
                    terminal_title: None,
                })
                .is_some(),
            "{provider} blocker fixture did not match"
        );
        assert_eq!(
            overlay.evaluate(BlockerInput {
                viewport: idle,
                terminal_title: None,
            }),
            None,
            "{provider} idle fixture was a false blocker"
        );
    }
}

#[test]
fn codex_title_can_add_needs_input_without_screen_text() {
    let overlay = BlockerOverlay::bundled(Provider::Codex).unwrap();
    let result = overlay
        .evaluate(BlockerInput {
            viewport: "",
            terminal_title: Some("[ . ] Action Required | blackpepper"),
        })
        .unwrap();
    assert_eq!(result.rule_id, "title_action_required");
}

#[test]
fn highest_priority_match_wins_without_returning_evidence() {
    let overlay = BlockerOverlay::from_toml(
        r#"
schema_version = 1
provider = "codex"
version = "1.0.0"

[[blockers]]
id = "low"
confidence = "medium"
priority = 1
contains_any = ["secret prompt"]

[[blockers]]
id = "high"
confidence = "high"
priority = 10
contains_all = ["secret", "prompt"]
"#,
    )
    .unwrap();
    let result = overlay
        .evaluate(BlockerInput {
            viewport: "secret prompt",
            terminal_title: None,
        })
        .unwrap();
    assert_eq!(result.rule_id, "high");
    assert_eq!(result.priority, 10);
    assert!(!serde_json::to_string(&result)
        .unwrap()
        .contains("secret prompt"));
}

#[test]
fn strict_schema_rejects_state_fields_and_unknown_keys() {
    let state_field = r#"
schema_version = 1
provider = "codex"
version = "1.0.0"

[[blockers]]
id = "bad"
confidence = "medium"
state = "done"
contains_any = ["finished"]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(state_field),
        Err(BlockerManifestError::InvalidToml(_))
    ));

    let unknown_root = r#"
schema_version = 1
provider = "codex"
version = "1.0.0"
fallback = "idle"

[[blockers]]
id = "bad"
confidence = "medium"
contains_any = ["finished"]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(unknown_root),
        Err(BlockerManifestError::InvalidToml(_))
    ));
}

#[test]
fn rule_confidence_is_required_in_pinned_manifests() {
    let missing_confidence = r#"
schema_version = 1
provider = "codex"
version = "1.0.0"

[[blockers]]
id = "approval"
contains_any = ["approve"]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(missing_confidence),
        Err(BlockerManifestError::InvalidToml(_))
    ));
}

#[test]
fn strict_schema_rejects_negative_only_and_invalid_regex_rules() {
    let negative_only = r#"
schema_version = 1
provider = "claude"
version = "1.0.0"

[[blockers]]
id = "negative_only"
confidence = "medium"
not_contains = ["idle"]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(negative_only),
        Err(BlockerManifestError::InvalidRule { .. })
    ));

    let invalid_regex = r#"
schema_version = 1
provider = "claude"
version = "1.0.0"

[[blockers]]
id = "invalid_regex"
confidence = "medium"
regex_any = ["("]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(invalid_regex),
        Err(BlockerManifestError::InvalidRegex { .. })
    ));
}

#[test]
fn strict_schema_rejects_bad_region_and_version() {
    let bad_region = r#"
schema_version = 1
provider = "opencode"
version = "1.0.0"

[[blockers]]
id = "bad_region"
confidence = "medium"
region = { kind = "bottom_lines" }
contains_any = ["permission"]
"#;
    assert!(matches!(
        BlockerOverlay::from_toml(bad_region),
        Err(BlockerManifestError::InvalidRule { .. })
    ));

    let bad_version = r#"
schema_version = 1
provider = "opencode"
version = "latest"

[[blockers]]
id = "rule"
confidence = "medium"
contains_any = ["permission"]
"#;
    assert_eq!(
        BlockerOverlay::from_toml(bad_version).unwrap_err(),
        BlockerManifestError::InvalidVersion
    );
}

#[test]
fn blockers_are_limited_to_the_selected_live_region() {
    let overlay = BlockerOverlay::bundled(Provider::Codex).unwrap();
    let stale = format!(
        "Press enter to confirm or esc to cancel\n{}",
        (0..20)
            .map(|index| format!("new output line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        overlay.evaluate(BlockerInput {
            viewport: &stale,
            terminal_title: None,
        }),
        None
    );
}
