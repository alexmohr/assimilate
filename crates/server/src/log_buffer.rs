// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::{Event, Subscriber, field::Visit};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

const DEFAULT_CAPACITY: usize = 2000;

/// A single log entry stored in the ring buffer.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LogEntry {
    /// When the log entry was created.
    pub timestamp: DateTime<Utc>,
    /// Log level (ERROR, WARN, INFO, DEBUG, TRACE).
    pub level: String,
    /// Tracing target / module path.
    pub target: String,
    /// Log message content.
    pub message: String,
}

/// An in-memory ring buffer of recent log entries.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
    capacity: usize,
}

impl LogBuffer {
    /// Create a new buffer with the given maximum capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    fn push(&self, entry: LogEntry) {
        let Ok(mut buf) = self.inner.lock() else {
            return;
        };
        if buf.len() >= self.capacity {
            buf.pop_front();
        }
        buf.push_back(entry);
    }

    /// Return the most recent entries, optionally filtered by minimum level and search text.
    #[must_use]
    pub fn entries(
        &self,
        limit: usize,
        min_level: Option<&str>,
        search: Option<&str>,
    ) -> Vec<LogEntry> {
        let Ok(buf) = self.inner.lock() else {
            return Vec::new();
        };
        buf.iter()
            .rev()
            .filter(|e| min_level.is_none_or(|lvl| level_matches(&e.level, lvl)))
            .filter(|e| {
                search.is_none_or(|q| {
                    let q = q.to_lowercase();
                    e.message.to_lowercase().contains(&q) || e.target.to_lowercase().contains(&q)
                })
            })
            .take(limit)
            .cloned()
            .collect()
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

fn level_matches(entry_level: &str, min_level: &str) -> bool {
    let entry_ord = level_ord(entry_level);
    let min_ord = level_ord(min_level);
    entry_ord <= min_ord
}

/// Ordering classification of a `tracing` level name. Only used to rank log
/// entries by severity for the `min_level` filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    Unknown,
}

impl From<&str> for LogLevel {
    fn from(level: &str) -> Self {
        match level.to_uppercase().as_str() {
            "ERROR" => Self::Error,
            "WARN" => Self::Warn,
            "INFO" => Self::Info,
            "DEBUG" => Self::Debug,
            "TRACE" => Self::Trace,
            _ => Self::Unknown,
        }
    }
}

fn level_ord(level: &str) -> u8 {
    match LogLevel::from(level) {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
        LogLevel::Trace => 4,
        LogLevel::Unknown => 5,
    }
}

struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
        }
    }
}

#[allow(
    unknown_lints,
    reason = "no_string_control_flow is a workspace-local dylint lint, unknown to plain \
              rustc/clippy"
)]
#[allow(
    no_string_control_flow,
    reason = "\"message\" is tracing's own reserved field name for the format-args value, not \
              domain state"
)]
impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" || self.message.is_empty() {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{}: {value}", field.name());
        }
    }
}

/// A tracing-subscriber layer that captures log events into the shared ring buffer.
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    /// Create a new layer that writes events into the given buffer.
    #[must_use]
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        let entry = LogEntry {
            timestamp: Utc::now(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
        };

        self.buffer.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let buf = LogBuffer::new(3);
        for i in 0..5 {
            buf.push(LogEntry {
                timestamp: Utc::now(),
                level: "INFO".to_string(),
                target: "test".to_string(),
                message: format!("msg {i}"),
            });
        }
        let entries = buf.entries(10, None, None);
        let [newest, middle, oldest] = entries.as_slice() else {
            panic!("expected exactly 3 entries, got {entries:?}");
        };
        assert_eq!(newest.message, "msg 4");
        assert_eq!(middle.message, "msg 3");
        assert_eq!(oldest.message, "msg 2");
    }

    #[test]
    fn filter_by_level() {
        let buf = LogBuffer::new(10);
        buf.push(LogEntry {
            timestamp: Utc::now(),
            level: "DEBUG".to_string(),
            target: "test".to_string(),
            message: "debug msg".to_string(),
        });
        buf.push(LogEntry {
            timestamp: Utc::now(),
            level: "ERROR".to_string(),
            target: "test".to_string(),
            message: "error msg".to_string(),
        });
        buf.push(LogEntry {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "test".to_string(),
            message: "info msg".to_string(),
        });

        let errors = buf.entries(10, Some("error"), None);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors.first().unwrap().message, "error msg");

        let warn_and_above = buf.entries(10, Some("warn"), None);
        assert_eq!(warn_and_above.len(), 1);

        let all = buf.entries(10, Some("debug"), None);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn filter_by_search() {
        let buf = LogBuffer::new(10);
        buf.push(LogEntry {
            timestamp: Utc::now(),
            level: "INFO".to_string(),
            target: "server::api".to_string(),
            message: "request handled".to_string(),
        });
        buf.push(LogEntry {
            timestamp: Utc::now(),
            level: "ERROR".to_string(),
            target: "server::db".to_string(),
            message: "connection failed".to_string(),
        });

        let results = buf.entries(10, None, Some("connection"));
        assert_eq!(results.len(), 1);
        assert_eq!(results.first().unwrap().message, "connection failed");

        let results = buf.entries(10, None, Some("api"));
        assert_eq!(results.len(), 1);
        assert_eq!(results.first().unwrap().target, "server::api");
    }
}
