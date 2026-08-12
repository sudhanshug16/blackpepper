use super::super::{ClientEvent, ClientState};
use crossterm::terminal::size;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
enum InputRead {
    Bytes(Vec<u8>),
    Idle,
    Closed,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum PollReadiness {
    Readable,
    Idle,
    Closed,
}

pub(super) fn flush_input_modes(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut ClientState,
) -> io::Result<()> {
    if state.pending_input_mode_bytes.is_empty() {
        return Ok(());
    }
    terminal
        .backend_mut()
        .write_all(&state.pending_input_mode_bytes)?;
    terminal.backend_mut().flush()?;
    state.pending_input_mode_bytes.clear();
    Ok(())
}

pub(super) fn spawn_input_thread(
    sender: Sender<ClientEvent>,
    shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut last_size = None;
        let mut pending_flush = false;
        while !shutdown.load(Ordering::Acquire) {
            if !forward_input_result(
                &sender,
                &mut pending_flush,
                read_raw_bytes(Duration::from_millis(25)),
            ) || !sync_resize(&sender, &mut last_size)
            {
                return;
            }
        }
    })
}

fn forward_input_result(
    sender: &Sender<ClientEvent>,
    pending_flush: &mut bool,
    result: io::Result<InputRead>,
) -> bool {
    match result {
        Ok(InputRead::Bytes(bytes)) => {
            *pending_flush = true;
            sender.send(ClientEvent::RawInput(bytes)).is_ok()
        }
        Ok(InputRead::Idle) if *pending_flush => {
            *pending_flush = false;
            sender.send(ClientEvent::InputFlush).is_ok()
        }
        Ok(InputRead::Idle) => true,
        Ok(InputRead::Closed) | Err(_) => {
            let _ = sender.send(ClientEvent::TerminalInputClosed);
            false
        }
    }
}

fn sync_resize(sender: &Sender<ClientEvent>, previous: &mut Option<(u16, u16)>) -> bool {
    let current = size().ok().map(|(columns, rows)| (rows, columns));
    if current.is_some() && current != *previous {
        *previous = current;
        return sender.send(ClientEvent::Resize).is_ok();
    }
    true
}

fn read_raw_bytes(timeout: Duration) -> io::Result<InputRead> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mut descriptor = libc::pollfd {
            fd: io::stdin().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe {
            libc::poll(
                &mut descriptor,
                1,
                timeout.as_millis().try_into().unwrap_or(i32::MAX),
            )
        };
        if result == 0
            || (result < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted)
        {
            return Ok(InputRead::Idle);
        }
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        match classify_poll_revents(descriptor.revents) {
            PollReadiness::Readable => {}
            PollReadiness::Idle => return Ok(InputRead::Idle),
            PollReadiness::Closed => return Ok(InputRead::Closed),
        }
    }
    #[cfg(not(unix))]
    if !crossterm::event::poll(timeout)? {
        return Ok(InputRead::Idle);
    }

    read_available(&mut io::stdin())
}

#[cfg(unix)]
fn classify_poll_revents(revents: libc::c_short) -> PollReadiness {
    if revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
        PollReadiness::Closed
    } else if revents & libc::POLLIN != 0 {
        // POLLHUP can accompany the final readable bytes. Consume them first;
        // the following zero-length read or HUP-only poll closes the reader.
        PollReadiness::Readable
    } else if revents & libc::POLLHUP != 0 {
        PollReadiness::Closed
    } else {
        PollReadiness::Idle
    }
}

fn read_available(reader: &mut impl Read) -> io::Result<InputRead> {
    let mut buffer = [0u8; 4096];
    let size = reader.read(&mut buffer)?;
    if size == 0 {
        Ok(InputRead::Closed)
    } else {
        Ok(InputRead::Bytes(buffer[..size].to_vec()))
    }
}

#[cfg(test)]
mod tests;
