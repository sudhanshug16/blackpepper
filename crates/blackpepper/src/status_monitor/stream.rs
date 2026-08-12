use std::borrow::Cow;
use std::io::{self, BufRead};
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use super::{BlockerTransition, StreamStats, ViewportBlockerMonitor};

/// A hard bound prevents a corrupted subscription from causing unbounded
/// allocation. Normal viewports are far smaller than two MiB.
pub const MAX_SUBSCRIPTION_LINE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum LineDisposition {
    Unchanged,
    OtherPane,
    UnknownEvent,
    Malformed,
    Oversize,
}

/// Consume Zellij's NDJSON stream until EOF.
///
/// Malformed, unknown, and oversized records are counted and skipped. Their
/// contents are never placed in an error or returned to the caller.
pub fn consume_subscription<R, C, E>(
    reader: R,
    monitor: &mut ViewportBlockerMonitor,
    now_ms: C,
    mut emit: E,
) -> io::Result<StreamStats>
where
    R: BufRead,
    C: FnMut() -> u64,
    E: FnMut(BlockerTransition),
{
    consume_subscription_fallible(reader, monitor, now_ms, |transition| {
        emit(transition);
        Ok(())
    })
}

/// Fallible variant used by the SSH helper so a closed client stream stops the
/// host-local Zellij subscription instead of leaving an orphan process.
pub fn consume_subscription_fallible<R, C, E>(
    reader: R,
    monitor: &mut ViewportBlockerMonitor,
    now_ms: C,
    emit: E,
) -> io::Result<StreamStats>
where
    R: BufRead,
    C: FnMut() -> u64,
    E: FnMut(BlockerTransition) -> io::Result<()>,
{
    consume_subscription_with(
        reader,
        now_ms,
        |line, observed_at_ms| Ok(process_bounded_line(line, monitor, observed_at_ms)),
        emit,
    )
}

/// Shared-monitor form used by the long-lived helper. The stream worker locks
/// only while reducing one already-bounded viewport record, allowing the
/// helper thread to change OpenCode authority between pane updates.
pub(crate) fn consume_subscription_shared_fallible<R, C, E>(
    reader: R,
    monitor: Arc<Mutex<ViewportBlockerMonitor>>,
    now_ms: C,
    emit: E,
) -> io::Result<StreamStats>
where
    R: BufRead,
    C: FnMut() -> u64,
    E: FnMut(BlockerTransition) -> io::Result<()>,
{
    consume_subscription_with(
        reader,
        now_ms,
        move |line, observed_at_ms| {
            let mut monitor = monitor
                .lock()
                .map_err(|_| io::Error::other("blocker monitor lock was poisoned"))?;
            Ok(process_bounded_line(line, &mut monitor, observed_at_ms))
        },
        emit,
    )
}

fn consume_subscription_with<R, C, P, E>(
    mut reader: R,
    mut now_ms: C,
    mut process: P,
    mut emit: E,
) -> io::Result<StreamStats>
where
    R: BufRead,
    C: FnMut() -> u64,
    P: FnMut(BoundedLine, u64) -> io::Result<ProcessedLine>,
    E: FnMut(BlockerTransition) -> io::Result<()>,
{
    let mut stats = StreamStats::default();
    loop {
        let Some(line) = read_bounded_line(&mut reader)? else {
            return Ok(stats);
        };
        stats.lines = stats.lines.saturating_add(1);
        let processed = process(line, now_ms())?;
        match processed {
            ProcessedLine::Transition(transition) => {
                stats.transitions = stats.transitions.saturating_add(1);
                emit(transition)?;
            }
            ProcessedLine::Disposition(LineDisposition::Malformed) => {
                stats.malformed = stats.malformed.saturating_add(1)
            }
            ProcessedLine::Disposition(LineDisposition::Oversize) => {
                stats.oversize = stats.oversize.saturating_add(1)
            }
            ProcessedLine::Disposition(LineDisposition::UnknownEvent) => {
                stats.unknown_events = stats.unknown_events.saturating_add(1)
            }
            ProcessedLine::Disposition(LineDisposition::OtherPane) => {
                stats.ignored_other_panes = stats.ignored_other_panes.saturating_add(1)
            }
            ProcessedLine::Disposition(LineDisposition::Unchanged) => {}
        }
    }
}

fn process_bounded_line(
    line: BoundedLine,
    monitor: &mut ViewportBlockerMonitor,
    observed_at_ms: u64,
) -> ProcessedLine {
    match line {
        BoundedLine::Oversize => ProcessedLine::Disposition(LineDisposition::Oversize),
        BoundedLine::Bytes(bytes) => match std::str::from_utf8(&bytes) {
            Ok(line) => process_line(line.trim_end_matches(['\r', '\n']), monitor, observed_at_ms),
            Err(_) => ProcessedLine::Disposition(LineDisposition::Malformed),
        },
    }
}

enum BoundedLine {
    Bytes(Vec<u8>),
    Oversize,
}

fn read_bounded_line(reader: &mut impl BufRead) -> io::Result<Option<BoundedLine>> {
    let mut bytes = Vec::new();
    let mut oversize = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return if bytes.is_empty() && !oversize {
                Ok(None)
            } else if oversize {
                Ok(Some(BoundedLine::Oversize))
            } else {
                Ok(Some(BoundedLine::Bytes(bytes)))
            };
        }
        let consumed = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        if !oversize {
            let remaining = MAX_SUBSCRIPTION_LINE_BYTES.saturating_sub(bytes.len());
            if consumed <= remaining {
                bytes.extend_from_slice(&buffer[..consumed]);
            } else {
                bytes.clear();
                oversize = true;
            }
        }
        let found_newline = buffer.get(consumed.saturating_sub(1)) == Some(&b'\n');
        reader.consume(consumed);
        if found_newline {
            return Ok(Some(if oversize {
                BoundedLine::Oversize
            } else {
                BoundedLine::Bytes(bytes)
            }));
        }
    }
}

enum ProcessedLine {
    Transition(BlockerTransition),
    Disposition(LineDisposition),
}

fn process_line(
    line: &str,
    monitor: &mut ViewportBlockerMonitor,
    observed_at_ms: u64,
) -> ProcessedLine {
    if line.trim().is_empty() {
        return ProcessedLine::Disposition(LineDisposition::Malformed);
    }
    let envelope: Envelope<'_> = match serde_json::from_str(line) {
        Ok(event) => event,
        Err(_) => return ProcessedLine::Disposition(LineDisposition::Malformed),
    };
    match envelope.event.as_ref() {
        "pane_update" => {
            let update: PaneUpdate<'_> = match serde_json::from_str(line) {
                Ok(update) => update,
                Err(_) => return ProcessedLine::Disposition(LineDisposition::Malformed),
            };
            if update.pane_id != monitor.zellij_pane_id() {
                return ProcessedLine::Disposition(LineDisposition::OtherPane);
            }
            let viewport = update.viewport.join("\n");
            transition_or_unchanged(monitor.observe(&viewport, None, observed_at_ms))
        }
        "pane_closed" => {
            let closed: PaneClosed<'_> = match serde_json::from_str(line) {
                Ok(closed) => closed,
                Err(_) => return ProcessedLine::Disposition(LineDisposition::Malformed),
            };
            if closed.pane_id != monitor.zellij_pane_id() {
                return ProcessedLine::Disposition(LineDisposition::OtherPane);
            }
            transition_or_unchanged(monitor.pane_closed(observed_at_ms))
        }
        _ => ProcessedLine::Disposition(LineDisposition::UnknownEvent),
    }
}

fn transition_or_unchanged(transition: Option<BlockerTransition>) -> ProcessedLine {
    transition.map_or(
        ProcessedLine::Disposition(LineDisposition::Unchanged),
        ProcessedLine::Transition,
    )
}

#[derive(Deserialize)]
struct Envelope<'a> {
    #[serde(borrow)]
    event: Cow<'a, str>,
}

#[derive(Deserialize)]
struct PaneUpdate<'a> {
    #[serde(borrow)]
    pane_id: Cow<'a, str>,
    #[serde(borrow)]
    viewport: Vec<Cow<'a, str>>,
}

#[derive(Deserialize)]
struct PaneClosed<'a> {
    #[serde(borrow)]
    pane_id: Cow<'a, str>,
}
