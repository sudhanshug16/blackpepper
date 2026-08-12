use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use portable_pty::PtySize;

use super::ssh_command::{self, ControlAction};
use super::{
    ConnectionState, ControlSocket, HostCommand, HostTransport, LocalForward, SshConfig,
    SshTransport,
};

fn arguments(spec: &super::ProcessSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|value| value.to_string_lossy().to_string())
        .collect()
}

#[test]
fn mux_children_are_normal_sessions_with_atomic_fail_closed_options() {
    let root = tempfile::tempdir().unwrap();
    let socket = ControlSocket::allocate(Some(root.path())).unwrap();
    let mut config = SshConfig::new("devbox");
    config.config_file = Some("/tmp/ssh config".into());
    let command = HostCommand::new("printf").args(["%s", "hello world"]);

    let spec = ssh_command::session_spec(&config, &socket, &command, true).unwrap();
    let args = arguments(&spec);

    for required in [
        "ControlMaster=no",
        "ControlPersist=no",
        "ProxyJump=none",
        "ProxyCommand=false",
        "CanonicalizeHostname=no",
        "BatchMode=yes",
    ] {
        assert!(args.iter().any(|argument| argument == required));
    }
    assert!(!args.iter().any(|argument| argument == "-O"));
    assert!(args.iter().any(|argument| argument == "-tt"));
    assert!(args.windows(2).any(|pair| pair == ["-e", "none"]));
    assert_eq!(args.last().unwrap(), "exec printf '%s' 'hello world'");

    let poison = args
        .iter()
        .position(|arg| arg == "ProxyCommand=false")
        .unwrap();
    let config_file = args.iter().position(|arg| arg == "-F").unwrap();
    assert!(
        poison < config_file,
        "safety options must win config parsing"
    );
}

#[test]
fn foreground_master_enables_master_mode_exactly_once() {
    let root = tempfile::tempdir().unwrap();
    let socket = ControlSocket::allocate(Some(root.path())).unwrap();
    let config = SshConfig::new("devbox");

    let spec = ssh_command::master_spec(&config, &socket).unwrap();
    let args = arguments(&spec);

    assert!(args.iter().any(|argument| argument == "ControlMaster=yes"));
    assert!(args.iter().any(|argument| argument == "-T"));
    assert!(args.iter().any(|argument| argument == "-N"));
    assert!(
        !args.iter().any(|argument| argument == "-M"),
        "a second master-mode request changes OpenSSH to ControlMaster=ask"
    );
    assert_eq!(args.last().unwrap(), "devbox");
}

#[test]
fn caller_cannot_override_the_owned_config_file() {
    let mut config = SshConfig::new("devbox");
    config.master_args.push("-F".into());
    config.master_args.push("/tmp/other-config".into());
    assert!(config.validate().is_err());
}

#[test]
fn forwarding_is_scoped_to_the_owned_control_socket() {
    let root = tempfile::tempdir().unwrap();
    let socket = ControlSocket::allocate(Some(root.path())).unwrap();
    let config = SshConfig::new("devbox");
    let forward = LocalForward::loopback(49152, 3000);

    let spec =
        ssh_command::control_spec(&config, &socket, ControlAction::Forward(&forward)).unwrap();
    let args = arguments(&spec);
    assert!(args.windows(2).any(|pair| pair == ["-O", "forward"]));
    assert!(args
        .windows(2)
        .any(|pair| { pair == ["-L", "127.0.0.1:49152:127.0.0.1:3000"] }));
    assert!(args.iter().any(|arg| arg == "ProxyCommand=false"));
}

#[test]
fn forwarding_keeps_an_exact_ipv6_remote_target() {
    let root = tempfile::tempdir().unwrap();
    let socket = ControlSocket::allocate(Some(root.path())).unwrap();
    let config = SshConfig::new("devbox");
    let forward = LocalForward {
        bind_address: "127.0.0.1".parse().unwrap(),
        local_port: 49152,
        remote_host: "::1".to_string(),
        remote_port: 3000,
    };

    let spec =
        ssh_command::control_spec(&config, &socket, ControlAction::Forward(&forward)).unwrap();
    let args = arguments(&spec);
    assert!(args
        .windows(2)
        .any(|pair| { pair == ["-L", "127.0.0.1:49152:[::1]:3000"] }));
}

#[cfg(unix)]
#[test]
fn master_reader_handoff_keeps_prompt_input_and_readiness_polling_working() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("ready");
    let script = temp.path().join("ssh-stub");
    let script_body = format!(
        "#!/bin/sh\ncase \" $* \" in\n  *\" -O check \"*) test -f '{}' ; exit $? ;;\n  *\" -O exit \"*) exit 0 ;;\nesac\nprintf 'Password: '\nIFS= read -r answer\ntest \"$answer\" = secret || exit 1\n: > '{}'\nwhile :; do sleep 1; done\n",
        marker.display(),
        marker.display()
    );
    fs::write(&script, script_body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions).unwrap();

    let mut config = SshConfig::new("stub-host");
    config.ssh_binary = script;
    config.control_root = Some(temp.path().join("control"));
    let mut transport = SshTransport::new(config).unwrap();
    transport.start_master(PtySize::default()).unwrap();

    let mut reader = transport.master_pty_mut().unwrap().take_reader().unwrap();
    assert!(transport.master_pty_mut().unwrap().take_reader().is_err());
    let mut prompt = [0u8; 10];
    reader.read_exact(&mut prompt).unwrap();
    assert_eq!(&prompt, b"Password: ");
    transport
        .master_pty_mut()
        .unwrap()
        .write_all(b"secret\n")
        .unwrap();
    fs::write(transport.control_socket_path().unwrap(), b"stub socket").unwrap();

    let mut state = ConnectionState::Connecting;
    for _ in 0..50 {
        state = transport.poll_connection().unwrap();
        if state == ConnectionState::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state, ConnectionState::Ready);
    transport.disconnect().unwrap();
}

#[cfg(unix)]
#[test]
fn session_spawn_preflights_master_and_refuses_a_missing_mux() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let check_marker = temp.path().join("check-called");
    let session_marker = temp.path().join("session-started");
    let script = temp.path().join("ssh-stub");
    let script_body = format!(
        "#!/bin/sh\ncase \" $* \" in\n  *\" -O check \"*) : > '{}'; exit 1 ;;\n  *\" -O exit \"*) exit 0 ;;\n  *\" ControlMaster=yes \"*) while :; do sleep 1; done ;;\nesac\n: > '{}'\nexit 0\n",
        check_marker.display(),
        session_marker.display(),
    );
    fs::write(&script, script_body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions).unwrap();

    let mut config = SshConfig::new("stub-host");
    config.ssh_binary = script;
    config.control_root = Some(temp.path().join("control"));
    let mut transport = SshTransport::new(config).unwrap();
    transport.start_master(PtySize::default()).unwrap();
    fs::write(transport.control_socket_path().unwrap(), b"stub socket").unwrap();
    let mut state = ConnectionState::Connecting;
    for _ in 0..50 {
        state = transport.poll_connection().unwrap();
        if state == ConnectionState::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(state, ConnectionState::Ready);
    assert!(!check_marker.exists());

    assert!(transport.spawn_exec(&HostCommand::new("true")).is_err());
    assert!(check_marker.exists());
    assert!(!session_marker.exists());
    assert_eq!(transport.state(), &ConnectionState::Failed { status: None });
}

#[cfg(unix)]
#[test]
fn ready_connection_polling_does_not_spawn_repeated_mux_checks() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let check_log = temp.path().join("checks");
    let script = temp.path().join("ssh-stub");
    let script_body = format!(
        "#!/bin/sh\ncase \" $* \" in\n  *\" -O check \"*) printf 'check\\n' >> '{}'; exit 0 ;;\n  *\" -O exit \"*) exit 0 ;;\n  *\" ControlMaster=yes \"*) while :; do sleep 1; done ;;\nesac\nexit 99\n",
        check_log.display(),
    );
    fs::write(&script, script_body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&script, permissions).unwrap();

    let mut config = SshConfig::new("stub-host");
    config.ssh_binary = script;
    config.control_root = Some(temp.path().join("control"));
    let mut transport = SshTransport::new(config).unwrap();
    transport.start_master(PtySize::default()).unwrap();
    fs::write(transport.control_socket_path().unwrap(), b"stub socket").unwrap();

    assert_eq!(transport.poll_connection().unwrap(), ConnectionState::Ready);
    for _ in 0..100 {
        assert_eq!(transport.poll_connection().unwrap(), ConnectionState::Ready);
    }

    assert!(
        !check_log.exists(),
        "render-thread readiness polling must not spawn a mux check"
    );
    transport.master_pty_mut().unwrap().kill().unwrap();
    let mut state = ConnectionState::Ready;
    for _ in 0..50 {
        state = transport.poll_connection().unwrap();
        if matches!(state, ConnectionState::Failed { .. }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(state, ConnectionState::Failed { .. }));
    assert!(!check_log.exists());
    transport.disconnect().unwrap();
}

#[cfg(unix)]
#[test]
fn control_socket_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let socket = ControlSocket::allocate(Some(root.path())).unwrap();
    let mode = fs::metadata(socket.directory())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
    assert!(socket.path().starts_with(root.path()));
    assert!(!Path::new(socket.path()).exists());
}
