use std::fmt;
use std::io::{Read, Write};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use super::{ProcessSpec, TransportError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyExit {
    pub success: bool,
    pub code: u32,
    pub signal: Option<String>,
}

/// A child attached to a local pseudo-terminal.
///
/// SSH PTY sessions still use a local PTY: the child is the system `ssh`
/// client, which forwards terminal size and byte streams to the remote PTY.
pub struct PtyProcess {
    master: Box<dyn MasterPty + Send>,
    reader: Option<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
}

impl fmt::Debug for PtyProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PtyProcess")
            .field("process_id", &self.process_id())
            .field("reader_taken", &self.reader.is_none())
            .finish_non_exhaustive()
    }
}

impl PtyProcess {
    pub(crate) fn spawn(spec: &ProcessSpec, size: PtySize) -> Result<Self, TransportError> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: size.rows.max(1),
                cols: size.cols.max(1),
                pixel_width: size.pixel_width,
                pixel_height: size.pixel_height,
            })
            .map_err(|error| TransportError::Pty(format!("failed to open PTY: {error}")))?;

        let mut command = CommandBuilder::new(&spec.program);
        command.args(&spec.args);
        if let Some(cwd) = &spec.cwd {
            command.cwd(cwd);
        }
        for (key, value) in &spec.env {
            command.env(key, value);
        }
        #[cfg(unix)]
        command.umask(spec.creation_umask.map(|mask| mask as libc::mode_t));

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TransportError::Pty(format!("failed to spawn PTY child: {error}")))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| TransportError::Pty(format!("failed to clone PTY reader: {error}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| TransportError::Pty(format!("failed to take PTY writer: {error}")))?;

        Ok(Self {
            master: pair.master,
            reader: Some(reader),
            writer,
            child: Some(child),
        })
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(|child| child.process_id())
    }

    /// The output reader may be moved to one background thread exactly once.
    pub fn take_reader(&mut self) -> Result<Box<dyn Read + Send>, TransportError> {
        self.reader
            .take()
            .ok_or_else(|| TransportError::Pty("PTY reader was already taken".to_string()))
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.writer
            .write_all(bytes)
            .and_then(|_| self.writer.flush())
            .map_err(|source| TransportError::io("failed to write to PTY", source))
    }

    pub fn resize(&self, size: PtySize) -> Result<(), TransportError> {
        self.master
            .resize(PtySize {
                rows: size.rows.max(1),
                cols: size.cols.max(1),
                pixel_width: size.pixel_width,
                pixel_height: size.pixel_height,
            })
            .map_err(|error| TransportError::Pty(format!("failed to resize PTY: {error}")))
    }

    pub fn try_wait(&mut self) -> Result<Option<PtyExit>, TransportError> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| TransportError::Pty("PTY child was already reaped".to_string()))?
            .try_wait()
            .map_err(|source| TransportError::io("failed to poll PTY child", source))?;
        Ok(status.map(|status| PtyExit {
            success: status.success(),
            code: status.exit_code(),
            signal: status.signal().map(str::to_string),
        }))
    }

    pub fn kill(&mut self) -> Result<(), TransportError> {
        self.child
            .as_mut()
            .ok_or_else(|| TransportError::Pty("PTY child was already reaped".to_string()))?
            .kill()
            .map_err(|source| TransportError::io("failed to terminate PTY child", source))
    }

    pub fn wait(&mut self) -> Result<PtyExit, TransportError> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| TransportError::Pty("PTY child was already reaped".to_string()))?
            .wait()
            .map_err(|source| TransportError::io("failed to wait for PTY child", source))?;
        self.child.take();
        Ok(PtyExit {
            success: status.success(),
            code: status.exit_code(),
            signal: status.signal().map(str::to_string),
        })
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        }
    }
}
