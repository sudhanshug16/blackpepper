use super::*;

#[derive(Default)]
struct RecordingWriter {
    bytes: Vec<u8>,
    writes: usize,
    fail_write: Option<usize>,
    fail_flush: bool,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.fail_write == Some(self.writes) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        } else {
            Ok(())
        }
    }
}

fn fully_armed_guard() -> TerminalSessionGuard {
    TerminalSessionGuard {
        input_modes_armed: true,
        raw_mode_armed: true,
        alternate_screen_armed: true,
        cursor_armed: true,
        flush_armed: true,
    }
}

#[test]
fn restore_is_conservative_and_idempotent() {
    let mut guard = fully_armed_guard();
    let mut writer = RecordingWriter::default();
    let mut raw_disables = 0;

    guard
        .restore_with(&mut writer, || {
            raw_disables += 1;
            Ok(())
        })
        .unwrap();
    guard
        .restore_with(&mut writer, || {
            raw_disables += 1;
            Ok(())
        })
        .unwrap();

    let expected = [
        CONSERVATIVE_INPUT_RESET,
        LEAVE_ALTERNATE_SCREEN,
        SHOW_CURSOR,
    ]
    .concat();
    assert_eq!(writer.bytes, expected);
    assert_eq!(raw_disables, 1);
    assert!(!guard.has_pending_cleanup());
}

#[test]
fn cleanup_failure_does_not_suppress_later_actions() {
    let mut guard = fully_armed_guard();
    let mut writer = RecordingWriter {
        fail_write: Some(1),
        ..RecordingWriter::default()
    };
    let mut raw_disables = 0;

    let error = guard
        .restore_with(&mut writer, || {
            raw_disables += 1;
            Err(io::Error::other("raw mode unavailable"))
        })
        .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(writer.bytes, [LEAVE_ALTERNATE_SCREEN, SHOW_CURSOR].concat());
    assert_eq!(raw_disables, 1);
    assert!(guard.input_modes_armed);
    assert!(guard.raw_mode_armed);
    assert!(!guard.alternate_screen_armed);
    assert!(!guard.cursor_armed);

    let mut retry = RecordingWriter::default();
    guard
        .restore_with(&mut retry, || {
            raw_disables += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(retry.bytes, CONSERVATIVE_INPUT_RESET);
    assert_eq!(raw_disables, 2);
    assert!(!guard.has_pending_cleanup());
}

#[test]
fn failed_setup_steps_remain_armed_for_rollback() {
    let mut guard = TerminalSessionGuard::new();
    let error = guard
        .enable_raw_mode_with(|| Err(io::Error::other("not a terminal")))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(guard.input_modes_armed);
    assert!(guard.raw_mode_armed);

    let mut setup_writer = RecordingWriter {
        fail_write: Some(1),
        ..RecordingWriter::default()
    };
    guard
        .enter_alternate_screen_with(&mut setup_writer, |writer| writer.write_all(b"ALT"))
        .unwrap_err();
    assert!(guard.alternate_screen_armed);
    assert!(guard.input_modes_armed);

    let mut cleanup_writer = RecordingWriter::default();
    guard.restore_with(&mut cleanup_writer, || Ok(())).unwrap();
    assert_eq!(
        cleanup_writer.bytes,
        [CONSERVATIVE_INPUT_RESET, LEAVE_ALTERNATE_SCREEN].concat()
    );
}

#[test]
fn entering_the_tui_enables_outer_focus_reports_before_any_workspace_attaches() {
    let mut guard = TerminalSessionGuard::new();
    let mut writer = RecordingWriter::default();

    guard
        .enter_alternate_screen_with(&mut writer, |writer| writer.write_all(b"ALT"))
        .unwrap();

    assert_eq!(
        writer.bytes,
        [b"ALT".as_slice(), ENABLE_FOCUS_REPORTING].concat()
    );
    assert!(guard.input_modes_armed);
    assert!(guard.alternate_screen_armed);
    guard.input_modes_armed = false;
    guard.alternate_screen_armed = false;
    guard.flush_armed = false;
}

#[test]
fn flush_failure_retries_every_uncertain_write() {
    let mut guard = fully_armed_guard();
    let mut failed_writer = RecordingWriter {
        fail_flush: true,
        ..RecordingWriter::default()
    };

    guard
        .restore_with(&mut failed_writer, || Ok(()))
        .unwrap_err();
    assert!(guard.input_modes_armed);
    assert!(guard.alternate_screen_armed);
    assert!(guard.cursor_armed);

    let mut retry = RecordingWriter::default();
    guard.restore_with(&mut retry, || Ok(())).unwrap();
    assert_eq!(
        retry.bytes,
        [
            CONSERVATIVE_INPUT_RESET,
            LEAVE_ALTERNATE_SCREEN,
            SHOW_CURSOR,
        ]
        .concat()
    );
    assert!(!guard.has_pending_cleanup());
}
