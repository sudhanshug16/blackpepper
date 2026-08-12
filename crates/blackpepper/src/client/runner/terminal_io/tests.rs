use super::*;
use std::io::{self, Cursor, Read};
use std::sync::mpsc;

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "test read failure",
        ))
    }
}

#[cfg(unix)]
#[test]
fn poll_revents_preserve_final_input_and_close_terminal_failures() {
    for event in [libc::POLLERR, libc::POLLNVAL] {
        assert_eq!(classify_poll_revents(event), PollReadiness::Closed);
        assert_eq!(
            classify_poll_revents(event | libc::POLLIN),
            PollReadiness::Closed
        );
    }
    assert_eq!(classify_poll_revents(libc::POLLIN), PollReadiness::Readable);
    assert_eq!(classify_poll_revents(libc::POLLHUP), PollReadiness::Closed);
    assert_eq!(
        classify_poll_revents(libc::POLLHUP | libc::POLLIN),
        PollReadiness::Readable
    );
    assert_eq!(classify_poll_revents(0), PollReadiness::Idle);
}

#[test]
fn zero_length_read_is_terminal_input_closure() {
    let mut input = Cursor::new(Vec::<u8>::new());

    assert_eq!(read_available(&mut input).unwrap(), InputRead::Closed);
}

#[test]
fn available_input_preserves_every_byte() {
    let mut input = Cursor::new(b"terminal bytes".to_vec());

    assert_eq!(
        read_available(&mut input).unwrap(),
        InputRead::Bytes(b"terminal bytes".to_vec())
    );
}

#[test]
fn closure_and_read_errors_each_emit_one_closed_event_then_stop() {
    let mut failing_reader = FailingReader;
    for result in [Ok(InputRead::Closed), read_available(&mut failing_reader)] {
        let (sender, receiver) = mpsc::channel();
        let mut pending_flush = true;

        assert!(!forward_input_result(&sender, &mut pending_flush, result));
        assert!(matches!(
            receiver.recv().unwrap(),
            ClientEvent::TerminalInputClosed
        ));
        assert!(receiver.try_recv().is_err());
    }
}
