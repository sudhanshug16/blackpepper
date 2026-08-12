use super::*;
use crate::transport::{
    CommandOutput, HostKind, LocalForward, ProcessSpec, PtyProcess, RunningCommand, TransportError,
};
use portable_pty::PtySize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
struct RecordedCommand {
    command: HostCommand,
    with_stdin: bool,
}

pub(super) struct RecordingTransport {
    data_home: PathBuf,
    home: PathBuf,
    path: String,
    commands: Vec<RecordedCommand>,
    pub(super) mutate_upload_source: Option<PathBuf>,
}

impl RecordingTransport {
    pub(super) fn new(root: &Path, data_home: PathBuf, path_directory: &Path) -> Self {
        Self {
            data_home,
            home: root.join("home"),
            path: format!("{}:/usr/bin:/bin", path_directory.display()),
            commands: Vec::new(),
            mutate_upload_source: None,
        }
    }

    fn process_spec(&self, command: &HostCommand) -> ProcessSpec {
        let mut spec = ProcessSpec::new(&command.program)
            .args(command.args.clone())
            .env("HOME", self.home.as_os_str())
            .env("XDG_DATA_HOME", self.data_home.as_os_str())
            .env("PATH", &self.path);
        if let Some(cwd) = &command.cwd {
            spec = spec.cwd(cwd);
        }
        for (key, value) in &command.env {
            spec = spec.env(key, value);
        }
        spec
    }

    fn is_environment_query(command: &HostCommand) -> bool {
        command.program == "sh"
            && command
                .args
                .get(1)
                .is_some_and(|script| script.contains("$(uname -s)"))
    }
}

impl HostTransport for RecordingTransport {
    fn kind(&self) -> HostKind {
        HostKind::Local
    }

    fn spawn_exec(&mut self, command: &HostCommand) -> Result<RunningCommand, TransportError> {
        self.commands.push(RecordedCommand {
            command: command.clone(),
            with_stdin: false,
        });
        RunningCommand::spawn(&self.process_spec(command), false)
    }

    fn spawn_exec_with_stdin(
        &mut self,
        command: &HostCommand,
    ) -> Result<RunningCommand, TransportError> {
        if let Some(path) = self.mutate_upload_source.take() {
            fs::write(path, b"corrupted after digest").unwrap();
        }
        self.commands.push(RecordedCommand {
            command: command.clone(),
            with_stdin: true,
        });
        RunningCommand::spawn(&self.process_spec(command), true)
    }

    fn exec(&mut self, command: &HostCommand) -> Result<CommandOutput, TransportError> {
        if Self::is_environment_query(command) {
            self.commands.push(RecordedCommand {
                command: command.clone(),
                with_stdin: false,
            });
            return Ok(CommandOutput {
                success: true,
                status: Some(0),
                stdout: format!("Linux\nx86_64\n{}\n", self.data_home.display()).into_bytes(),
                stderr: Vec::new(),
            });
        }
        self.spawn_exec(command)?.wait_with_output()
    }

    fn exec_timeout(
        &mut self,
        command: &HostCommand,
        timeout: std::time::Duration,
    ) -> Result<CommandOutput, TransportError> {
        if Self::is_environment_query(command) {
            return self.exec(command);
        }
        self.spawn_exec(command)?.wait_with_output_timeout(timeout)
    }

    fn attach_pty(
        &mut self,
        _command: &HostCommand,
        _size: PtySize,
    ) -> Result<PtyProcess, TransportError> {
        Err(TransportError::Unsupported("not used by helper tests"))
    }

    fn forward_local_port(
        &mut self,
        _forward: LocalForward,
    ) -> Result<LocalForward, TransportError> {
        Err(TransportError::Unsupported("not used by helper tests"))
    }

    fn cancel_local_forward(&mut self, _forward: &LocalForward) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("not used by helper tests"))
    }
}

pub(super) fn write_helper(path: &Path, build_id: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "#!/bin/sh\n[ \"${{1:-}}\" = --version ] || exit 64\nprintf '%s\\n' 'bp-host {build_id}'\n"
        ),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn linux_managed_path(data_home: &Path) -> PathBuf {
    helper_install_directory(
        data_home,
        crate::BUILD_ID,
        SidecarTarget::LinuxX86_64.triple(),
    )
    .join("bp-host")
}

fn assert_remote_shell_round_trip(command: &HostCommand) {
    let line = command.remote_shell_line().unwrap();
    let words = shell_words::split(line.strip_prefix("exec ").unwrap()).unwrap();
    let expected = std::iter::once(command.program.clone())
        .chain(command.args.iter().cloned())
        .collect::<Vec<_>>();
    assert_eq!(words, expected);
}

#[test]
fn path_helper_remains_the_first_exact_build_preference() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    let path_helper = bin.join("bp-host");
    write_helper(&path_helper, crate::BUILD_ID);
    let mut transport = RecordingTransport::new(root.path(), root.path().join("data"), &bin);

    let resolved =
        find_helper_with(&mut transport, |_| panic!("managed upload was consulted")).unwrap();

    assert_eq!(resolved, path_helper.to_string_lossy());
    assert!(!transport
        .commands
        .iter()
        .any(|record| RecordingTransport::is_environment_query(&record.command)));
}

#[test]
fn reconnect_reuses_the_exact_cached_managed_helper() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let data_home = root.path().join("remote-data");
    let managed = linux_managed_path(&data_home);
    write_helper(&managed, crate::BUILD_ID);
    let mut transport = RecordingTransport::new(root.path(), data_home, &bin);

    let first = find_helper_with(&mut transport, |_| panic!("cached helper was uploaded")).unwrap();
    let second =
        find_helper_with(&mut transport, |_| panic!("cached helper was uploaded")).unwrap();

    assert_eq!(first, managed.to_string_lossy());
    assert_eq!(second, first);
    assert_eq!(
        transport
            .commands
            .iter()
            .filter(|record| record.command.program == first)
            .count(),
        2
    );
    assert!(!transport.commands.iter().any(|record| record.with_stdin));
}

#[test]
fn wrong_cached_build_uploads_once_without_shell_injection() {
    let root = tempfile::tempdir().unwrap();
    let injection_marker = root.path().join("INJECTED");
    let data_home = root.path().join(format!(
        "remote data; touch {}; #",
        injection_marker.display()
    ));
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let managed = linux_managed_path(&data_home);
    write_helper(&managed, "wrong-build");
    let bundled = root.path().join("bundled-bp-host");
    write_helper(&bundled, crate::BUILD_ID);
    let mut transport = RecordingTransport::new(root.path(), data_home, &bin);

    let resolved = find_helper_with(&mut transport, |_| Ok(bundled.clone())).unwrap();

    assert_eq!(resolved, managed.to_string_lossy());
    assert!(!injection_marker.exists());
    let upload = transport
        .commands
        .iter()
        .position(|record| record.with_stdin)
        .expect("wrong cached build must trigger an upload");
    let first_managed_probe = transport
        .commands
        .iter()
        .position(|record| record.command.program == resolved)
        .unwrap();
    assert!(first_managed_probe < upload);
    assert_eq!(
        transport
            .commands
            .iter()
            .filter(|record| record.with_stdin)
            .count(),
        1
    );
    for record in &transport.commands {
        assert_remote_shell_round_trip(&record.command);
    }
    let output = std::process::Command::new(&managed)
        .arg("--version")
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("bp-host {}", crate::BUILD_ID)
    );
}

#[test]
fn invalid_cached_helper_also_falls_through_to_upload() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let data_home = root.path().join("remote-data");
    let managed = linux_managed_path(&data_home);
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    fs::write(&managed, "#!/bin/sh\nexit 70\n").unwrap();
    fs::set_permissions(&managed, fs::Permissions::from_mode(0o700)).unwrap();
    let bundled = root.path().join("bundled-bp-host");
    write_helper(&bundled, crate::BUILD_ID);
    let mut transport = RecordingTransport::new(root.path(), data_home, &bin);

    let resolved = find_helper_with(&mut transport, |_| Ok(bundled)).unwrap();

    assert_eq!(resolved, managed.to_string_lossy());
    assert_eq!(
        transport
            .commands
            .iter()
            .filter(|record| record.with_stdin)
            .count(),
        1
    );
}

#[test]
fn development_and_release_helpers_use_distinct_remote_paths() {
    let data_home = Path::new("/home/test/.local/share");
    let release = helper_install_directory(data_home, "0.1.66", "x86_64-unknown-linux-musl");
    let development = helper_install_directory(
        data_home,
        "0.1.66-dev.abcdef.20260811",
        "x86_64-unknown-linux-musl",
    );

    assert_ne!(release, development);
    assert!(release.ends_with("0.1.66/x86_64-unknown-linux-musl"));
    assert!(development.ends_with("0.1.66-dev.abcdef.20260811/x86_64-unknown-linux-musl"));
}
