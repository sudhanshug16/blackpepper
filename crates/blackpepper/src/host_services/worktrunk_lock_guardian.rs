//! Parent-side control channel for the process that keeps a repository lock
//! alive across abrupt `bp-host` termination.

use std::io::{Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::Duration;

#[path = "worktrunk_lock_guardian_child.rs"]
mod child;

pub(super) const OP_REGISTER: u8 = 1;
pub(super) const OP_COMPLETE: u8 = 2;
pub(super) const OP_RELEASE: u8 = 3;
pub(super) const RESPONSE_READY: u8 = 10;
pub(super) const RESPONSE_OK: u8 = 11;
pub(super) const RESPONSE_BUSY: u8 = 12;
pub(super) const RESPONSE_INVALID: u8 = 13;
#[cfg(test)]
pub(super) const OP_TEST_HOLD_LOCK: u8 = 99;

pub(in crate::host_services) struct RegisteredProcessGroup<'a> {
    guardian: &'a LockGuardian,
    process_group: libc::pid_t,
    finished: bool,
}

impl RegisteredProcessGroup<'_> {
    /// Finish a dispatched command. The guardian terminates any descendants
    /// that retained the Worktrunk process group before acknowledging this.
    pub(in crate::host_services) fn finish(mut self) -> Result<(), String> {
        let result = self.guardian.request(OP_COMPLETE, self.process_group);
        self.finished = true;
        result
    }
}

impl Drop for RegisteredProcessGroup<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.guardian.request(OP_COMPLETE, self.process_group);
        }
    }
}

pub(super) struct LockGuardian {
    channel: Mutex<UnixStream>,
    pid: libc::pid_t,
}

impl LockGuardian {
    pub(super) fn spawn(lock_fd: RawFd) -> Result<Self, String> {
        let mut sockets = [-1; 2];
        if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, sockets.as_mut_ptr()) }
            == -1
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        for fd in sockets {
            if !set_close_on_exec(fd) {
                close_pair(sockets);
                return Err(std::io::Error::last_os_error().to_string());
            }
        }

        let pid = unsafe { libc::fork() };
        if pid == -1 {
            close_pair(sockets);
            return Err(std::io::Error::last_os_error().to_string());
        }
        if pid == 0 {
            // The child closes inherited runtime descriptors and then uses
            // only async-signal-safe libc operations. It never returns into
            // the potentially multi-threaded Rust process after this fork.
            unsafe {
                libc::close(sockets[0]);
                child::run(sockets[1], lock_fd);
            }
        }

        unsafe {
            libc::close(sockets[1]);
        }
        let mut channel = unsafe { UnixStream::from_raw_fd(sockets[0]) };
        if let Err(error) = configure_channel(&channel) {
            reap_guardian(pid);
            return Err(error);
        }
        let mut ready = [0_u8; 1];
        if let Err(error) = channel.read_exact(&mut ready) {
            reap_guardian(pid);
            return Err(format!("lock guardian did not start: {error}"));
        }
        if ready[0] != RESPONSE_READY {
            reap_guardian(pid);
            return Err("lock guardian returned an invalid startup response".to_owned());
        }
        Ok(Self {
            channel: Mutex::new(channel),
            pid,
        })
    }

    pub(super) fn register(
        &self,
        process_group: libc::pid_t,
    ) -> Result<RegisteredProcessGroup<'_>, String> {
        self.request(OP_REGISTER, process_group)?;
        Ok(RegisteredProcessGroup {
            guardian: self,
            process_group,
            finished: false,
        })
    }

    fn request(&self, operation: u8, process_group: libc::pid_t) -> Result<(), String> {
        if process_group < 0 {
            return Err("Invalid Worktrunk process-group identity.".to_owned());
        }
        let mut request = [0_u8; 8];
        request[0] = operation;
        request[4..8].copy_from_slice(&process_group.to_ne_bytes());
        let mut channel = self
            .channel
            .lock()
            .map_err(|_| "Worktrunk lock guardian channel was poisoned.".to_owned())?;
        channel
            .write_all(&request)
            .map_err(|error| format!("Worktrunk lock guardian disconnected: {error}"))?;
        let mut response = [0_u8; 1];
        channel
            .read_exact(&mut response)
            .map_err(|error| format!("Worktrunk lock guardian did not respond: {error}"))?;
        match response[0] {
            RESPONSE_OK => Ok(()),
            RESPONSE_BUSY => Err(
                "A Worktrunk child process survived forced cleanup; the repository lock remains held."
                    .to_owned(),
            ),
            _ => Err("Worktrunk lock guardian rejected the process group.".to_owned()),
        }
    }

    pub(super) fn release(&self) {
        if self.request(OP_RELEASE, 0).is_err() {
            return;
        }
        // If an unkillable child remains, leave the guardian alive holding its
        // inherited lock instead of making another mutation look safe.
        for _ in 0..100 {
            let mut status = 0;
            let waited = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if waited == self.pid || waited == -1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    pub(super) fn hold_lock_after_release_for_test(&self) -> Result<(), String> {
        self.request(OP_TEST_HOLD_LOCK, 0)
    }
}

fn configure_channel(channel: &UnixStream) -> Result<(), String> {
    channel
        .set_read_timeout(Some(Duration::from_secs(2)))
        .and_then(|()| channel.set_write_timeout(Some(Duration::from_secs(2))))
        .map_err(|error| error.to_string())
}

fn set_close_on_exec(fd: RawFd) -> bool {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    flags != -1 && unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } != -1
}

fn close_pair(sockets: [RawFd; 2]) {
    unsafe {
        libc::close(sockets[0]);
        libc::close(sockets[1]);
    }
}

fn reap_guardian(pid: libc::pid_t) {
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
    }
}
