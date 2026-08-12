use std::io::{self, Read, Write};
use std::time::Duration;

use super::SidecarError;

pub(super) const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const READ_CANCEL_INTERVAL: Duration = Duration::from_secs(1);
const DOWNLOAD_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);

pub trait SidecarDownloader: Send + Sync {
    fn download(&self, url: &str, destination: &mut dyn Write) -> Result<(), SidecarError>;
}

/// HTTPS downloader used for immutable GitHub release assets.
#[derive(Debug, Clone)]
pub struct HttpDownloader {
    agent: ureq::Agent,
}

impl Default for HttpDownloader {
    fn default() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            // Wake periodically so a reconnect/shutdown cancellation token is
            // observed even while a server stops sending bytes.
            .timeout_read(READ_CANCEL_INTERVAL)
            .timeout_write(Duration::from_secs(120))
            .redirects(5)
            .build();
        Self { agent }
    }
}

impl SidecarDownloader for HttpDownloader {
    fn download(&self, url: &str, destination: &mut dyn Write) -> Result<(), SidecarError> {
        if !url.starts_with("https://") {
            return Err(SidecarError::Download {
                url: url.to_string(),
                message: "managed sidecars require HTTPS release URLs".to_string(),
            });
        }
        let response = self
            .agent
            .get(url)
            .set(
                "User-Agent",
                concat!("blackpepper/", env!("CARGO_PKG_VERSION")),
            )
            .call()
            .map_err(|error| SidecarError::Download {
                url: url.to_string(),
                message: error.to_string(),
            })?;
        let mut source = response.into_reader().take(MAX_ARCHIVE_BYTES + 1);
        let copied =
            copy_cancellable(&mut source, destination).map_err(|error| SidecarError::Download {
                url: url.to_string(),
                message: error.to_string(),
            })?;
        if copied > MAX_ARCHIVE_BYTES {
            return Err(SidecarError::ArchiveTooLarge {
                asset: url.to_string(),
                limit: MAX_ARCHIVE_BYTES,
            });
        }
        Ok(())
    }
}

fn copy_cancellable(reader: &mut impl Read, writer: &mut dyn Write) -> io::Result<u64> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0_u64;
    let started = std::time::Instant::now();
    loop {
        if super::CommandCancellation::scope_is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "managed sidecar download was cancelled",
            ));
        }
        if started.elapsed() >= DOWNLOAD_TOTAL_TIMEOUT {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "managed sidecar download exceeded its 120-second deadline",
            ));
        }
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) && started.elapsed() < DOWNLOAD_TOTAL_TIMEOUT =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        if count == 0 {
            return Ok(copied);
        }
        writer.write_all(&buffer[..count])?;
        copied = copied.saturating_add(count as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TimedOutReader;

    impl Read for TimedOutReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_millis(5));
            Err(io::ErrorKind::TimedOut.into())
        }
    }

    #[test]
    fn managed_downloads_reject_plain_http() {
        let mut destination = Vec::new();
        assert!(matches!(
            HttpDownloader::default().download("http://example.invalid/sidecar", &mut destination),
            Err(SidecarError::Download { .. })
        ));
        assert!(destination.is_empty());
    }

    #[test]
    fn stalled_download_copy_observes_restore_cancellation() {
        let cancellation = super::super::CommandCancellation::default();
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || {
            let mut reader = TimedOutReader;
            let mut destination = Vec::new();
            worker_cancellation.scoped(|| copy_cancellable(&mut reader, &mut destination))
        });
        std::thread::sleep(Duration::from_millis(20));
        cancellation.cancel();

        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }
}
