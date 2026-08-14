use std::collections::BTreeMap;

use crate::providers::runtime::ProviderLaunch;
use crate::transport::HostCommand;

use super::super::terminal_identity;

pub(super) fn apply_agent_environment(
    launch: &mut ProviderLaunch,
    workspace_env: BTreeMap<String, String>,
    terminal_identity_supported: bool,
) {
    apply_agent_environment_with(
        launch,
        workspace_env,
        terminal_identity_supported,
        terminal_identity::apply,
    );
}

pub(super) fn apply_agent_environment_with(
    launch: &mut ProviderLaunch,
    workspace_env: BTreeMap<String, String>,
    terminal_identity_supported: bool,
    apply_terminal_identity: impl FnOnce(&mut BTreeMap<String, String>),
) {
    for (key, value) in workspace_env {
        launch.env.entry(key).or_insert(value);
    }
    if terminal_identity_supported {
        apply_terminal_identity(&mut launch.env);
    }
}

pub(super) fn initial_agent_command(launch: &ProviderLaunch) -> HostCommand {
    let mut arguments = launch
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    arguments.push(launch.program.clone());
    arguments.extend(launch.args.iter().cloned());
    HostCommand::new("env").args(arguments)
}
