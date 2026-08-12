use super::HelperResponse;
use std::{
    error::Error,
    fmt,
    io::{self, BufRead, Write},
};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

pub(super) fn write_response(
    writer: &mut impl Write,
    response: HelperResponse,
) -> Result<(), ProtocolError> {
    serde_json::to_writer(&mut *writer, &response)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub(super) enum LineRead {
    Eof,
    Line(Vec<u8>),
    TooLong,
}

pub(super) fn read_bounded_line(reader: &mut impl BufRead) -> Result<LineRead, io::Error> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut saw_bytes = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if !saw_bytes {
                Ok(LineRead::Eof)
            } else if too_long {
                Ok(LineRead::TooLong)
            } else {
                Ok(LineRead::Line(line))
            };
        }
        saw_bytes = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if !too_long {
            if line.len() + consumed > MAX_REQUEST_BYTES {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..consumed]);
            }
        }
        reader.consume(consumed);
        if newline.is_some() {
            if too_long {
                return Ok(LineRead::TooLong);
            }
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(LineRead::Line(line));
        }
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "helper protocol I/O error: {error}"),
            Self::Json(error) => write!(formatter, "helper protocol JSON error: {error}"),
        }
    }
}

impl Error for ProtocolError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
        }
    }
}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
