use std::ffi::OsStr;
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};

mod timeout;

pub(super) use timeout::run_bounded_timeout;

#[cfg(unix)]
use super::worktrunk_lock::{RegisteredProcessGroup, RepositoryLock};
#[cfg(unix)]
use crate::transport::{ProcessSpec, PtyExit, PtyProcess, TransportError};
#[cfg(unix)]
use portable_pty::PtySize;
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;

#[cfg(unix)]
const GATED_EXEC: &str = "IFS= read -r bp_gate || exit 125\n\
    [ \"$bp_gate\" = \"$1\" ] || exit 125\n\
    shift\n\
    exec \"$@\" </dev/null\n";

#[cfg(unix)]
const GATED_PTY_EXEC: &str = "IFS= read -r bp_gate || exit 125\n\
    [ \"$bp_gate\" = \"$1\" ] || exit 125\n\
    shift\n\
    exec \"$@\"\n";

#[derive(Debug)]
pub(super) struct BoundedOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

/// Runs a fixed executable with already-separated arguments. Both pipes are
/// drained concurrently so a noisy hook cannot deadlock the transient helper.
pub(super) fn run_bounded<I, S>(program: &OsStr, args: I) -> io::Result<BoundedOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_and_drain(stdout));
    let stderr_reader = std::thread::spawn(move || read_and_drain(stderr));
    let status = child.wait()?;
    let (stdout, stdout_truncated) = join_reader(stdout_reader)?;
    let (stderr, stderr_truncated) = join_reader(stderr_reader)?;
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
    })
}

/// Runs Worktrunk in a gated process group registered with the repository
/// lock guardian. Registration happens before the gate is opened, so abrupt
/// helper death cannot leave an untracked hook tree mutating after unlock.
#[cfg(unix)]
pub(super) fn run_bounded_guarded<I, S>(
    lock: &RepositoryLock,
    program: &OsStr,
    args: I,
) -> io::Result<BoundedOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let gate = uuid::Uuid::new_v4().to_string();
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(GATED_EXEC)
        .arg("bp-worktrunk-supervisor")
        .arg(&gate)
        .arg(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_and_drain(stdout));
    let stderr_reader = std::thread::spawn(move || read_and_drain(stderr));
    let registration = match lock.register_process_group(child.id() as libc::pid_t) {
        Ok(registration) => registration,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
            return Err(io::Error::other(error));
        }
    };
    let gate_result = child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(format!("{gate}\n").as_bytes());
    if let Err(error) = gate_result {
        let _ = child.kill();
        let _ = child.wait();
        let _ = registration.finish();
        let _ = join_reader(stdout_reader);
        let _ = join_reader(stderr_reader);
        return Err(error);
    }

    let status = child.wait();
    // Complete before joining output readers: a hook descendant may retain a
    // pipe after Worktrunk exits, and the guardian must close that tree first.
    let cleanup = registration.finish();
    let (stdout, stdout_truncated) = join_reader(stdout_reader)?;
    let (stderr, stderr_truncated) = join_reader(stderr_reader)?;
    let status = status?;
    cleanup.map_err(io::Error::other)?;
    #[cfg(test)]
    if std::env::var_os("BLACKPEPPER_TEST_FAIL_AFTER_EXEC")
        .is_some_and(|path| std::path::Path::new(&path).exists())
    {
        return Err(io::Error::other(
            "injected failure after guarded process dispatch",
        ));
    }
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
    })
}

#[cfg(not(unix))]
pub(super) fn run_bounded_guarded<I, S>(
    _lock: &super::worktrunk_lock::RepositoryLock,
    _program: &OsStr,
    _args: I,
) -> io::Result<BoundedOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "guarded Worktrunk execution requires Unix process groups",
    ))
}

/// Interactive counterpart to [`run_bounded_guarded`]. The small fixed shell
/// consumes a private gate in the PTY before execing Worktrunk, preserving the
/// real terminal required by `wt config state add` without an argv shell.
#[cfg(unix)]
pub(super) struct GuardedPtyProcess<'a> {
    process: PtyProcess,
    registration: Option<RegisteredProcessGroup<'a>>,
}

#[cfg(unix)]
impl<'a> GuardedPtyProcess<'a> {
    pub(super) fn spawn(
        lock: &'a RepositoryLock,
        spec: &ProcessSpec,
        size: PtySize,
    ) -> Result<Self, String> {
        let gate = uuid::Uuid::new_v4().to_string();
        let mut arguments = vec![
            OsString::from("-c"),
            OsString::from(GATED_PTY_EXEC),
            OsString::from("bp-worktrunk-supervisor"),
            OsString::from(&gate),
            spec.program.as_os_str().to_owned(),
        ];
        arguments.extend(spec.args.iter().cloned());
        let mut wrapper = ProcessSpec::new("/bin/sh").args(arguments);
        if let Some(cwd) = &spec.cwd {
            wrapper = wrapper.cwd(cwd);
        }
        for (key, value) in &spec.env {
            wrapper = wrapper.env(key, value);
        }
        let mut process = PtyProcess::spawn(&wrapper, size)
            .map_err(|error| format!("Could not start guarded Worktrunk PTY: {error}"))?;
        let process_group = process
            .process_id()
            .ok_or_else(|| "Guarded Worktrunk PTY did not expose a process ID.".to_owned())?
            as libc::pid_t;
        let registration = match lock.register_process_group(process_group) {
            Ok(registration) => registration,
            Err(error) => {
                let _ = process.kill();
                let _ = process.wait();
                return Err(error);
            }
        };
        if let Err(error) = process.write_all(format!("{gate}\n").as_bytes()) {
            let _ = process.kill();
            let _ = process.wait();
            let _ = registration.finish();
            return Err(format!(
                "Could not open guarded Worktrunk PTY gate: {error}"
            ));
        }
        Ok(Self {
            process,
            registration: Some(registration),
        })
    }

    pub(super) fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, TransportError> {
        self.process.take_reader()
    }

    pub(super) fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.process.write_all(bytes)
    }

    pub(super) fn wait(&mut self) -> Result<PtyExit, String> {
        let exit = self
            .process
            .wait()
            .map_err(|error| format!("Could not wait for Worktrunk approval: {error}"));
        let cleanup = self
            .registration
            .take()
            .expect("guarded PTY registration")
            .finish();
        let exit = exit?;
        cleanup?;
        Ok(exit)
    }
}

#[cfg(unix)]
impl Drop for GuardedPtyProcess<'_> {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _ = self.process.kill();
            let _ = self.process.wait();
            let _ = registration.finish();
        }
    }
}

fn read_and_drain(mut reader: impl Read) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok((retained, truncated));
        }
        let available = MAX_CAPTURE_BYTES.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(available)]);
        truncated |= read > available;
    }
}

fn join_reader(
    handle: std::thread::JoinHandle<io::Result<(Vec<u8>, bool)>>,
) -> io::Result<(Vec<u8>, bool)> {
    handle
        .join()
        .map_err(|_| io::Error::other("process output reader panicked"))?
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
