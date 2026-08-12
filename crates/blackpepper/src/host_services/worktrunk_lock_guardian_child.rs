//! Allocation-free child-side repository-lock and process-group state machine.

#[cfg(test)]
use super::OP_TEST_HOLD_LOCK;
use super::{
    OP_COMPLETE, OP_REGISTER, OP_RELEASE, RESPONSE_BUSY, RESPONSE_INVALID, RESPONSE_OK,
    RESPONSE_READY,
};
use std::os::fd::RawFd;

/// This function never returns and deliberately avoids Rust runtime services
/// after `fork`; its inherited lock descriptor is closed only after all known
/// Worktrunk process groups are gone.
pub(super) unsafe fn run(channel_fd: RawFd, lock_fd: RawFd) -> ! {
    let saved_lock = libc::fcntl(lock_fd, libc::F_DUPFD, 10);
    let saved_channel = libc::fcntl(channel_fd, libc::F_DUPFD, 10);
    if saved_lock == -1 || saved_channel == -1 {
        libc::_exit(120);
    }
    if libc::dup2(saved_lock, 3) == -1 || libc::dup2(saved_channel, 4) == -1 {
        libc::_exit(121);
    }
    close_unrelated_fds();
    let _ = libc::setsid();
    libc::signal(libc::SIGHUP, libc::SIG_IGN);
    libc::signal(libc::SIGINT, libc::SIG_IGN);
    libc::signal(libc::SIGQUIT, libc::SIG_IGN);
    libc::signal(libc::SIGTERM, libc::SIG_IGN);

    if !write_all(4, &[RESPONSE_READY]) {
        libc::_exit(122);
    }
    let mut active_groups = [0 as libc::pid_t; 16];
    #[cfg(test)]
    let mut hold_lock_after_release = false;
    #[cfg(not(test))]
    let hold_lock_after_release = false;
    loop {
        let mut request = [0_u8; 8];
        match read_exact(4, &mut request) {
            0 => {
                if hold_lock_after_release {
                    for _ in 0..200 {
                        sleep_poll();
                    }
                }
                cleanup_all_until_gone(&active_groups);
                libc::_exit(0);
            }
            value if value < 0 => {
                cleanup_all_until_gone(&active_groups);
                libc::_exit(123);
            }
            _ => {}
        }
        let process_group =
            libc::pid_t::from_ne_bytes([request[4], request[5], request[6], request[7]]);
        let response = match request[0] {
            #[cfg(test)]
            OP_TEST_HOLD_LOCK => {
                hold_lock_after_release = true;
                RESPONSE_OK
            }
            OP_REGISTER if process_group > 1 => register(&mut active_groups, process_group),
            OP_COMPLETE => complete(&mut active_groups, process_group),
            #[cfg(test)]
            OP_RELEASE if hold_lock_after_release => RESPONSE_BUSY,
            OP_RELEASE
                if active_groups.iter().all(|group| *group == 0) && !hold_lock_after_release =>
            {
                let _ = write_all(4, &[RESPONSE_OK]);
                libc::_exit(0);
            }
            OP_RELEASE => {
                if cleanup_all_once(&mut active_groups) {
                    let _ = write_all(4, &[RESPONSE_OK]);
                    libc::_exit(0);
                }
                RESPONSE_BUSY
            }
            _ => RESPONSE_INVALID,
        };
        if !write_all(4, &[response]) {
            cleanup_all_until_gone(&active_groups);
            libc::_exit(0);
        }
    }
}

unsafe fn register(groups: &mut [libc::pid_t], process_group: libc::pid_t) -> u8 {
    if let Some(slot) = groups.iter_mut().find(|slot| **slot == 0) {
        *slot = process_group;
        RESPONSE_OK
    } else {
        RESPONSE_BUSY
    }
}

unsafe fn complete(groups: &mut [libc::pid_t], process_group: libc::pid_t) -> u8 {
    let Some(slot) = groups.iter_mut().find(|slot| **slot == process_group) else {
        return RESPONSE_INVALID;
    };
    if terminate_group(*slot) {
        *slot = 0;
        RESPONSE_OK
    } else {
        RESPONSE_BUSY
    }
}

#[cfg(target_os = "linux")]
unsafe fn close_unrelated_fds() {
    libc::close(0);
    libc::close(1);
    libc::close(2);
    if libc::syscall(libc::SYS_close_range, 5_u32, u32::MAX, 0_u32) == -1 {
        close_fd_range();
    }
}

#[cfg(target_os = "macos")]
unsafe fn close_unrelated_fds() {
    libc::close(0);
    libc::close(1);
    libc::close(2);
    close_fd_range();
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
unsafe fn close_unrelated_fds() {
    libc::close(0);
    libc::close(1);
    libc::close(2);
    close_fd_range();
}

unsafe fn close_fd_range() {
    let maximum = libc::sysconf(libc::_SC_OPEN_MAX).clamp(5, 65_536);
    for fd in 5..maximum {
        libc::close(fd as RawFd);
    }
}

unsafe fn read_exact(fd: RawFd, buffer: &mut [u8]) -> i32 {
    let mut offset = 0;
    while offset < buffer.len() {
        let read = libc::read(
            fd,
            buffer[offset..].as_mut_ptr().cast(),
            buffer.len() - offset,
        );
        if read == 0 {
            return 0;
        }
        if read == -1 {
            if current_errno() == libc::EINTR {
                continue;
            }
            return -1;
        }
        offset += read as usize;
    }
    1
}

unsafe fn write_all(fd: RawFd, buffer: &[u8]) -> bool {
    let mut offset = 0;
    while offset < buffer.len() {
        let written = libc::write(fd, buffer[offset..].as_ptr().cast(), buffer.len() - offset);
        if written == -1 {
            if current_errno() == libc::EINTR {
                continue;
            }
            return false;
        }
        offset += written as usize;
    }
    true
}

unsafe fn terminate_group(process_group: libc::pid_t) -> bool {
    if process_group <= 1 || !group_exists(process_group) {
        return true;
    }
    libc::kill(-process_group, libc::SIGTERM);
    if wait_group_gone(process_group, 25) {
        return true;
    }
    libc::kill(-process_group, libc::SIGKILL);
    wait_group_gone(process_group, 100)
}

unsafe fn cleanup_all_once(process_groups: &mut [libc::pid_t]) -> bool {
    let mut clean = true;
    for process_group in process_groups {
        if *process_group > 1 {
            if terminate_group(*process_group) {
                *process_group = 0;
            } else {
                clean = false;
            }
        }
    }
    clean
}

unsafe fn cleanup_all_until_gone(process_groups: &[libc::pid_t]) {
    while process_groups
        .iter()
        .any(|process_group| *process_group > 1 && !terminate_group(*process_group))
    {
        sleep_poll();
    }
}

unsafe fn wait_group_gone(process_group: libc::pid_t, polls: usize) -> bool {
    for _ in 0..polls {
        if !group_exists(process_group) {
            return true;
        }
        sleep_poll();
    }
    !group_exists(process_group)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn group_exists(process_group: libc::pid_t) -> bool {
    libc::kill(-process_group, 0) == 0 || current_errno() == libc::EPERM
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
unsafe fn group_exists(process_group: libc::pid_t) -> bool {
    libc::kill(-process_group, 0) == 0 || current_errno() == libc::EPERM
}

#[cfg(any(target_os = "linux", target_os = "android"))]
unsafe fn current_errno() -> i32 {
    *libc::__errno_location()
}

#[cfg(any(
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
unsafe fn current_errno() -> i32 {
    *libc::__error()
}

unsafe fn sleep_poll() {
    let requested = libc::timespec {
        tv_sec: 0,
        tv_nsec: 10_000_000,
    };
    libc::nanosleep(&requested, std::ptr::null_mut());
}
