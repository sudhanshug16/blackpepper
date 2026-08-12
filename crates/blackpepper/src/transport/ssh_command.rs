use std::ffi::OsString;
use std::net::IpAddr;

use super::{ControlSocket, HostCommand, LocalForward, ProcessSpec, SshConfig, TransportError};

#[derive(Debug, Clone, Copy)]
pub(crate) enum ControlAction<'a> {
    Check,
    Forward(&'a LocalForward),
    Cancel(&'a LocalForward),
}

pub(crate) fn master_spec(
    config: &SshConfig,
    socket: &ControlSocket,
) -> Result<ProcessSpec, TransportError> {
    config.validate()?;
    let mut arguments = Vec::new();
    push_option(&mut arguments, "StrictHostKeyChecking", "ask");
    push_option(&mut arguments, "BatchMode", "no");
    push_option(&mut arguments, "ControlMaster", "yes");
    push_option(&mut arguments, "ControlPersist", "no");
    push_option(&mut arguments, "ClearAllForwardings", "yes");
    push_option(&mut arguments, "PermitLocalCommand", "no");
    push_option(&mut arguments, "ForkAfterAuthentication", "no");
    append_config_file(config, &mut arguments);
    arguments.extend(config.master_args.iter().cloned());
    // Do not also pass -M. OpenSSH treats two master-mode requests as
    // ControlMaster=ask, which makes unattended mux channels fail with
    // "Master refused session request" (and normally fall back directly).
    arguments.extend([
        OsString::from("-N"),
        OsString::from("-T"),
        OsString::from("-S"),
        socket.path().as_os_str().to_owned(),
        OsString::from("--"),
        OsString::from(&config.destination),
    ]);
    Ok(ProcessSpec::new(&config.ssh_binary).args(arguments))
}

pub(crate) fn session_spec(
    config: &SshConfig,
    socket: &ControlSocket,
    command: &HostCommand,
    pty: bool,
) -> Result<ProcessSpec, TransportError> {
    let remote_command = command.remote_shell_line()?;
    session_spec_line(config, socket, remote_command, pty)
}

pub(crate) fn session_spec_line(
    config: &SshConfig,
    socket: &ControlSocket,
    remote_command: String,
    pty: bool,
) -> Result<ProcessSpec, TransportError> {
    config.validate()?;
    if remote_command.contains('\0') {
        return Err(TransportError::InvalidCommand(
            "remote command must not contain NUL bytes".to_string(),
        ));
    }
    let mut arguments = fail_closed_arguments(config);
    push_option(&mut arguments, "ClearAllForwardings", "yes");
    push_option(&mut arguments, "RemoteCommand", "none");
    push_option(&mut arguments, "SessionType", "default");
    push_option(&mut arguments, "StdinNull", "no");
    arguments.extend([OsString::from("-S"), socket.path().as_os_str().to_owned()]);
    if pty {
        arguments.extend([
            OsString::from("-tt"),
            OsString::from("-e"),
            OsString::from("none"),
        ]);
    } else {
        arguments.push(OsString::from("-T"));
    }
    arguments.extend([
        OsString::from("--"),
        OsString::from(&config.destination),
        OsString::from(remote_command),
    ]);
    Ok(ProcessSpec::new(&config.ssh_binary).args(arguments))
}

pub(crate) fn control_spec(
    config: &SshConfig,
    socket: &ControlSocket,
    action: ControlAction<'_>,
) -> Result<ProcessSpec, TransportError> {
    config.validate()?;
    let (operation, forward) = match action {
        ControlAction::Check => ("check", None),
        ControlAction::Forward(forward) => {
            forward.validate()?;
            ("forward", Some(forward))
        }
        ControlAction::Cancel(forward) => {
            forward.validate()?;
            ("cancel", Some(forward))
        }
    };

    let mut arguments = fail_closed_arguments(config);
    push_option(
        &mut arguments,
        "ClearAllForwardings",
        if forward.is_some() { "no" } else { "yes" },
    );
    push_option(&mut arguments, "ExitOnForwardFailure", "yes");
    arguments.extend([
        OsString::from("-S"),
        socket.path().as_os_str().to_owned(),
        OsString::from("-O"),
        OsString::from(operation),
    ]);
    if let Some(forward) = forward {
        arguments.push(OsString::from("-L"));
        arguments.push(OsString::from(format_forward(forward)));
    }
    arguments.extend([OsString::from("--"), OsString::from(&config.destination)]);
    Ok(ProcessSpec::new(&config.ssh_binary).args(arguments))
}

fn fail_closed_arguments(config: &SshConfig) -> Vec<OsString> {
    let mut arguments = Vec::new();
    // These must precede config loading: OpenSSH uses the first value obtained
    // for these options. `ProxyCommand=false` is the final guard for the narrow
    // race where the readiness check succeeds but the master disappears before
    // this child opens its mux session. `-O proxy` is deliberately not used:
    // that mode exposes OpenSSH's mux proxy protocol on stdio, not a command
    // session. With an ordinary `-S` child, a missing mux would normally fall
    // back to a fresh SSH connection; the false proxy makes that fallback fail.
    push_option(&mut arguments, "ControlMaster", "no");
    push_option(&mut arguments, "ControlPersist", "no");
    push_option(&mut arguments, "ProxyJump", "none");
    push_option(&mut arguments, "ProxyCommand", "false");
    push_option(&mut arguments, "CanonicalizeHostname", "no");
    push_option(&mut arguments, "BatchMode", "yes");
    push_option(&mut arguments, "PermitLocalCommand", "no");
    push_option(&mut arguments, "ForkAfterAuthentication", "no");
    append_config_file(config, &mut arguments);
    arguments
}

fn append_config_file(config: &SshConfig, arguments: &mut Vec<OsString>) {
    if let Some(config_file) = &config.config_file {
        arguments.push(OsString::from("-F"));
        arguments.push(config_file.as_os_str().to_owned());
    }
}

fn push_option(arguments: &mut Vec<OsString>, name: &str, value: &str) {
    arguments.push(OsString::from("-o"));
    arguments.push(OsString::from(format!("{name}={value}")));
}

fn format_forward(forward: &LocalForward) -> String {
    format!(
        "{}:{}:{}:{}",
        format_ip(forward.bind_address),
        forward.local_port,
        format_host(&forward.remote_host),
        forward.remote_port
    )
}

fn format_ip(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{address}]"),
    }
}

fn format_host(host: &str) -> String {
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}
