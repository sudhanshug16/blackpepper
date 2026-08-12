use crate::zellij::ZellijError;
use std::fmt;
use std::io;

#[derive(Debug)]
pub enum HostSubscriptionError {
    Zellij(ZellijError),
    Spawn(io::Error),
    MissingStdout,
    Stream(io::Error),
    Wait(io::Error),
    Exited(Option<i32>),
    WorkerPanicked,
}

impl From<ZellijError> for HostSubscriptionError {
    fn from(error: ZellijError) -> Self {
        Self::Zellij(error)
    }
}

impl fmt::Display for HostSubscriptionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zellij(error) => write!(formatter, "invalid Zellij subscription: {error}"),
            Self::Spawn(error) => write!(formatter, "could not start Zellij subscription: {error}"),
            Self::MissingStdout => {
                formatter.write_str("Zellij subscription stdout was unavailable")
            }
            Self::Stream(error) => {
                write!(formatter, "Zellij subscription stream failed: {error}")
            }
            Self::Wait(error) => write!(formatter, "could not reap Zellij subscription: {error}"),
            Self::Exited(status) => write!(
                formatter,
                "Zellij subscription exited unsuccessfully (status {status:?})"
            ),
            Self::WorkerPanicked => formatter.write_str("Zellij subscription worker panicked"),
        }
    }
}

impl std::error::Error for HostSubscriptionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Zellij(error) => Some(error),
            Self::Spawn(error) | Self::Stream(error) | Self::Wait(error) => Some(error),
            Self::MissingStdout | Self::Exited(_) | Self::WorkerPanicked => None,
        }
    }
}
