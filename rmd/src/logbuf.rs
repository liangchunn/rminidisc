//! An in-memory `log` backend for the TUI.
//!
//! `env_logger` writes to stderr, which corrupts the full-screen TUI. Instead
//! the TUI installs [`TuiLogger`], which appends formatted records to a shared,
//! bounded ring buffer that the log panel renders. The max level is adjustable
//! at runtime so the in-app level modal can change verbosity live.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Maximum number of retained log lines.
const CAPACITY: usize = 2000;

/// A single captured log entry.
#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub target: String,
    pub message: String,
}

/// Shared, bounded buffer of captured log entries.
#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    fn push(&self, entry: LogEntry) {
        if let Ok(mut q) = self.inner.lock() {
            if q.len() == CAPACITY {
                q.pop_front();
            }
            q.push_back(entry);
        }
    }

    /// Returns a snapshot copy of the most recent `max` entries (oldest first).
    pub fn snapshot(&self, max: usize) -> Vec<LogEntry> {
        match self.inner.lock() {
            Ok(q) => {
                let start = q.len().saturating_sub(max);
                q.iter().skip(start).cloned().collect()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Total number of entries currently retained.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|q| q.len()).unwrap_or(0)
    }
}

/// The global logger backing the TUI. The runtime level lives in an atomic so
/// the modal can change it without re-installing the logger.
struct TuiLogger {
    buffer: LogBuffer,
    level: AtomicUsize,
}

static LOGGER: OnceLock<TuiLogger> = OnceLock::new();

fn level_to_usize(level: LevelFilter) -> usize {
    level as usize
}

fn usize_to_filter(value: usize) -> LevelFilter {
    match value {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

impl Log for TuiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let max = usize_to_filter(self.level.load(Ordering::Relaxed));
        metadata.level() <= max
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        self.buffer.push(LogEntry {
            level: record.level(),
            target: record.target().to_string(),
            message: format!("{}", record.args()),
        });
    }

    fn flush(&self) {}
}

/// Installs the TUI logger and returns a handle to its buffer plus a controller
/// for adjusting the level at runtime. Safe to call once; subsequent calls
/// return clones of the already-installed buffer/controller.
pub fn install(initial: LevelFilter) -> (LogBuffer, LevelControl) {
    let logger = LOGGER.get_or_init(|| TuiLogger {
        buffer: LogBuffer::default(),
        level: AtomicUsize::new(level_to_usize(initial)),
    });
    // Set the global max to Trace; per-record filtering is done in `enabled`
    // against the runtime atomic so the modal can raise the level later.
    let _ = log::set_logger(logger);
    log::set_max_level(LevelFilter::Trace);
    logger.level.store(level_to_usize(initial), Ordering::Relaxed);
    (
        logger.buffer.clone(),
        LevelControl {
            level: &logger.level,
        },
    )
}

/// Runtime controller for the logger's max level.
#[derive(Clone, Copy)]
pub struct LevelControl {
    level: &'static AtomicUsize,
}

impl LevelControl {
    pub fn set(&self, level: LevelFilter) {
        self.level.store(level_to_usize(level), Ordering::Relaxed);
    }

    pub fn get(&self) -> LevelFilter {
        usize_to_filter(self.level.load(Ordering::Relaxed))
    }
}
