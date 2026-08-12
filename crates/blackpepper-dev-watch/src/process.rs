use crate::config::Config;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) struct BuildProcess {
    child: Child,
    build_id: String,
}

impl BuildProcess {
    pub fn start(config: &Config, sequence: u64) -> Result<Self, String> {
        let build_id = config.build_id(sequence);
        append_log(
            &config.log,
            &format!("\n=== source build {} ({build_id}) ===", unix_timestamp()),
        )?;
        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.log)
            .map_err(|error| format!("could not open rebuild log: {error}"))?;
        let stderr = stdout
            .try_clone()
            .map_err(|error| format!("could not clone rebuild log: {error}"))?;
        let mut command = Command::new(&config.cargo);
        command
            .current_dir(&config.root)
            .args([
                "build",
                "--manifest-path",
                config.root.join("Cargo.toml").to_string_lossy().as_ref(),
                "--target-dir",
                config.target_dir.to_string_lossy().as_ref(),
                "--target",
                &config.host_target,
                "-p",
                "blackpepper",
                "--bin",
                "bp",
                "--bin",
                "bp-host",
            ])
            .env("BLACKPEPPER_BUILD_ID", &build_id)
            .env("BLACKPEPPER_SOURCE_WATCH_BUILD", "1")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .process_group(0);
        let child = command
            .spawn()
            .map_err(|error| format!("could not start temporary source build: {error}"))?;
        Ok(Self { child, build_id })
    }

    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        self.child
            .try_wait()
            .map_err(|error| format!("could not observe temporary source build: {error}"))
    }

    pub fn cancel(&mut self, grace: Duration) -> Result<(), String> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        signal_process_group(self.child.id(), libc::SIGTERM)?;
        if wait_for_exit(&mut self.child, grace)?.is_some() {
            return Ok(());
        }
        signal_process_group(self.child.id(), libc::SIGKILL)?;
        wait_for_exit(&mut self.child, grace)?
            .map(|_| ())
            .ok_or_else(|| "temporary source build did not stop after SIGKILL".to_owned())
    }
}

pub(crate) struct ClientProcess {
    child: Child,
}

impl ClientProcess {
    pub fn launch(binary: &Path, cwd: &Path) -> Result<Self, String> {
        let child = Command::new(binary)
            .current_dir(cwd)
            .spawn()
            .map_err(|error| format!("could not launch {}: {error}", binary.display()))?;
        Ok(Self { child })
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        self.child
            .try_wait()
            .map_err(|error| format!("could not observe source-run Blackpepper: {error}"))
    }

    pub fn stop(&mut self, grace: Duration, signal_first: bool) -> Result<(), String> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        if signal_first {
            signal_process(self.child.id(), libc::SIGTERM)?;
        }
        if wait_for_exit(&mut self.child, grace)?.is_some() {
            return Ok(());
        }
        // Blackpepper deliberately treats the second termination signal as an
        // immediate exit once cleanup has begun.
        signal_process(self.child.id(), libc::SIGTERM)?;
        wait_for_exit(&mut self.child, grace)?
            .map(|_| ())
            .ok_or_else(|| "source-run Blackpepper did not release its lifecycle lock".to_owned())
    }
}

pub(crate) fn reset_log(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create rebuild log directory: {error}"))?;
    }
    File::create(path)
        .and_then(|mut file| writeln!(file, "blackpepper temporary source-build log"))
        .map_err(|error| format!("could not initialize rebuild log: {error}"))
}

pub(crate) fn append_log(path: &Path, message: &str) -> Result<(), String> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("could not open rebuild log: {error}"))?;
    writeln!(log, "{message}").map_err(|error| format!("could not write rebuild log: {error}"))
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<Option<ExitStatus>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not wait for child process: {error}"))?
        {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn signal_process(pid: u32, signal: libc::c_int) -> Result<(), String> {
    let result = unsafe { libc::kill(pid as libc::pid_t, signal) };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "could not signal source-run client: {}",
            std::io::Error::last_os_error()
        ))
    }
}

fn signal_process_group(pid: u32, signal: libc::c_int) -> Result<(), String> {
    let result = unsafe { libc::kill(-(pid as libc::pid_t), signal) };
    if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(format!(
            "could not signal temporary source build: {}",
            std::io::Error::last_os_error()
        ))
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
