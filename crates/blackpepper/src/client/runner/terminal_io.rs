use super::super::{ClientEvent, ClientState};
use crossterm::terminal::size;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Read, Write};
use std::sync::mpsc::Sender;
use std::time::Duration;

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

pub(super) fn spawn_input_thread(sender: Sender<ClientEvent>) {
    std::thread::spawn(move || {
        let mut last_size = None;
        let mut pending_flush = false;
        loop {
            let sent = match read_raw_bytes(Duration::from_millis(25)) {
                Ok(Some(bytes)) => {
                    pending_flush = true;
                    sender.send(ClientEvent::RawInput(bytes)).is_ok()
                }
                Ok(None) if pending_flush => {
                    pending_flush = false;
                    sender.send(ClientEvent::InputFlush).is_ok()
                }
                Ok(None) => true,
                Err(_) => false,
            };
            if !sent || !sync_resize(&sender, &mut last_size) {
                return;
            }
        }
    });
}

fn sync_resize(sender: &Sender<ClientEvent>, previous: &mut Option<(u16, u16)>) -> bool {
    let current = size().ok().map(|(columns, rows)| (rows, columns));
    if current.is_some() && current != *previous {
        *previous = current;
        return sender.send(ClientEvent::Resize).is_ok();
    }
    true
}

fn read_raw_bytes(timeout: Duration) -> io::Result<Option<Vec<u8>>> {
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
            return Ok(None);
        }
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if descriptor.revents & libc::POLLIN == 0 {
            return Ok(None);
        }
    }
    #[cfg(not(unix))]
    if !crossterm::event::poll(timeout)? {
        return Ok(None);
    }

    let mut buffer = [0u8; 4096];
    let size = io::stdin().read(&mut buffer)?;
    Ok((size != 0).then(|| buffer[..size].to_vec()))
}
