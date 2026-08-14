use super::*;
use crate::client::runtime::terminal_identity;
use crate::providers::runtime::ProviderLaunch;
use std::collections::BTreeMap;
use std::path::Path;

fn launch(provider: ProviderKind) -> ProviderLaunch {
    build_launch(
        provider,
        WorkspaceId::new(),
        AgentRunId::new(),
        PaneId::new(),
        Path::new("/opt/blackpepper/bp-provider-event"),
        Path::new("/opt/blackpepper/integrations"),
    )
    .unwrap()
}

fn apply_workspace_and_terminal(
    launch: &mut ProviderLaunch,
    workspace_env: BTreeMap<String, String>,
    terminal_identity_supported: bool,
    terminal_program: &str,
    terminal_version: &str,
) {
    apply_agent_environment_with(
        launch,
        workspace_env,
        terminal_identity_supported,
        |environment| {
            terminal_identity::apply_with(environment, |key| match key {
                "TERM_PROGRAM" => Some(terminal_program.to_owned()),
                "TERM_PROGRAM_VERSION" => Some(terminal_version.to_owned()),
                _ => None,
            });
        },
    );
}

#[test]
fn all_agent_commands_receive_launch_terminal_identity_and_config_keeps_precedence() {
    for provider in [
        ProviderKind::Codex,
        ProviderKind::Claude,
        ProviderKind::OpenCode,
    ] {
        let mut inherited = launch(provider);
        apply_workspace_and_terminal(
            &mut inherited,
            BTreeMap::from([("PROJECT_VALUE".to_owned(), "kept".to_owned())]),
            true,
            "ghostty",
            "1.3.1",
        );
        let command = initial_agent_command(&inherited);
        assert_eq!(command.program, "env");
        assert!(command.args.contains(&"TERM_PROGRAM=ghostty".to_owned()));
        assert!(command
            .args
            .contains(&"TERM_PROGRAM_VERSION=1.3.1".to_owned()));
        assert!(command.args.contains(&"PROJECT_VALUE=kept".to_owned()));

        let mut configured = launch(provider);
        apply_workspace_and_terminal(
            &mut configured,
            BTreeMap::from([
                ("TERM_PROGRAM".to_owned(), "configured-terminal".to_owned()),
                (
                    "TERM_PROGRAM_VERSION".to_owned(),
                    "configured-version".to_owned(),
                ),
            ]),
            true,
            "ghostty",
            "1.3.1",
        );
        let command = initial_agent_command(&configured);
        assert!(command
            .args
            .contains(&"TERM_PROGRAM=configured-terminal".to_owned()));
        assert!(command
            .args
            .contains(&"TERM_PROGRAM_VERSION=configured-version".to_owned()));
        assert!(!command.args.contains(&"TERM_PROGRAM=ghostty".to_owned()));
    }
}

#[test]
fn stock_zellij_does_not_advertise_an_unforwardable_terminal_protocol() {
    let mut agent = launch(ProviderKind::Codex);

    apply_workspace_and_terminal(
        &mut agent,
        BTreeMap::from([("PROJECT_VALUE".to_owned(), "kept".to_owned())]),
        false,
        "ghostty",
        "1.3.1",
    );

    let command = initial_agent_command(&agent);
    assert!(command.args.contains(&"PROJECT_VALUE=kept".to_owned()));
    assert!(!command
        .args
        .iter()
        .any(|argument| argument.starts_with("TERM_PROGRAM=")));
}
