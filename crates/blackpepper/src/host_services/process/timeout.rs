use std::ffi::OsStr;
use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use super::{join_reader, read_and_drain, BoundedOutput};

/// Bounded-output variant for probing an executable that may be corrupt or
/// user-controlled. On Unix the probe gets its own process group so a child it
/// spawns cannot retain the output pipes or survive the deadline.
pub(in crate::host_services) fn run_bounded_timeout<I, S>(
    program: &OsStr,
    args: I,
    timeout: Duration,
) -> io::Result<BoundedOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let process_id = child.id();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_and_drain(stdout));
    let stderr_reader = std::thread::spawn(move || read_and_drain(stderr));
    let started = Instant::now();

    let status = loop {
        if let Some(status) = child.try_wait()? {
            // Close pipes inherited by descendants before joining the
            // drainers. Observation children must not outlive their probe.
            kill_probe_group(process_id, &mut child);
            break status;
        }
        if started.elapsed() >= timeout {
            kill_probe_group(process_id, &mut child);
            let _ = child.wait();
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("executable probe exceeded {} ms", timeout.as_millis()),
            ));
        }
        std::thread::sleep(
            Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
        );
    };
    let (stdout, stdout_truncated) = join_reader(stdout_reader)?;
    let (stderr, stderr_truncated) = join_reader(stderr_reader)?;
    Ok(BoundedOutput {
        status,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
    })
}

#[cfg(unix)]
fn kill_probe_group(process_id: u32, _child: &mut std::process::Child) {
    let _ = unsafe { libc::kill(-(process_id as libc::pid_t), libc::SIGKILL) };
}

#[cfg(not(unix))]
fn kill_probe_group(_process_id: u32, child: &mut std::process::Child) {
    let _ = child.kill();
}
