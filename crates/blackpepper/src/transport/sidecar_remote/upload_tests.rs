use super::*;
use crate::transport::{ProcessSpec, RunningCommand};
use std::io::Cursor;

struct ShortWriter {
    bytes: Vec<u8>,
    maximum_write: usize,
}

#[test]
fn upload_deadline_scales_for_large_binaries() {
    assert_eq!(upload_deadline(8 * 1024 * 1024), Duration::from_secs(120));
    assert_eq!(upload_deadline(64 * 1024 * 1024), Duration::from_secs(512));
}

impl Write for ShortWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let accepted = buffer.len().min(self.maximum_write);
        self.bytes.extend_from_slice(&buffer[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn upload_copy_tracks_every_partial_write() {
    let input = vec![0x5a; UPLOAD_BUFFER_BYTES + 37];
    let mut reader = Cursor::new(input.clone());
    let mut writer = ShortWriter {
        bytes: Vec::new(),
        maximum_write: 3,
    };
    let progress = AtomicU64::new(0);

    let copied =
        copy_with_progress(&mut reader, &mut writer, &progress, &AtomicBool::new(false)).unwrap();

    assert_eq!(copied, input.len() as u64);
    assert_eq!(progress.load(Ordering::Relaxed), copied);
    assert_eq!(writer.bytes, input);
}

#[cfg(unix)]
#[test]
fn blocked_upload_pipe_cancels_promptly_and_joins_writer() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("large-helper");
    std::fs::write(&local, vec![0x5a; 8 * 1024 * 1024]).unwrap();
    // This command deliberately never reads stdin, so the upload fills the
    // local pipe and exercises the blocked-writer cancellation path.
    let child = RunningCommand::spawn(
        &ProcessSpec::new("sh").args(["-c", "trap '' TERM; exec sleep 30"]),
        true,
    )
    .unwrap();
    let cancellation = CommandCancellation::default();
    let worker_cancellation = cancellation.clone();
    let local_for_worker = local.clone();
    let worker = thread::spawn(move || {
        worker_cancellation.scoped(|| upload_file_to_child(child, &local_for_worker))
    });
    thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();

    assert!(matches!(error, SidecarInstallError::UploadCancelled { .. }));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "a blocked upload writer outlived cancellation"
    );
}

#[test]
fn eof_wait_honors_scoped_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let local = temp.path().join("tiny-helper");
    std::fs::write(&local, b"payload").unwrap();
    let child = RunningCommand::spawn(
        &ProcessSpec::new("sh").args(["-c", "cat >/dev/null; exec sleep 30"]),
        true,
    )
    .unwrap();
    let cancellation = CommandCancellation::default();
    let worker_cancellation = cancellation.clone();
    let worker =
        thread::spawn(move || worker_cancellation.scoped(|| upload_file_to_child(child, &local)));
    thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    cancellation.cancel();
    let error = worker.join().unwrap().unwrap_err();

    assert!(matches!(
        error,
        SidecarInstallError::Transport(crate::transport::TransportError::CommandCancelled { .. })
    ));
    assert!(started.elapsed() < Duration::from_secs(3));
}
