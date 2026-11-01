//! One logger for everything: a file in the install folder, an in-memory ring the UI
//! can fetch, a `log-line` event for live tailing, and optional stdout echo for CLI runs.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use log::{Level, LevelFilter, Log, Metadata, Record};
use tauri::{AppHandle, Emitter};

const RING_CAPACITY: usize = 2000;
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

struct Logger {
    file: Mutex<Option<File>>,
    ring: Mutex<VecDeque<String>>,
    app: Mutex<Option<AppHandle>>,
    echo_stdout: bool,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        let target = record.target();
        // Only our own output; hyper/reqwest/tao chatter stays out of the user-facing log.
        if !(target.starts_with("stlauncher") || target == "sillytavern") {
            return;
        }
        let now = chrono::Local::now();
        let line = if target == "sillytavern" {
            format!("{} {}", now.format("%H:%M:%S"), record.args())
        } else {
            format!("{} [{}] {}", now.format("%H:%M:%S"), record.level(), record.args())
        };
        if self.echo_stdout {
            println!("{line}");
        }
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = writeln!(file, "{} {line}", now.format("%Y-%m-%d"));
            }
        }
        if let Ok(mut ring) = self.ring.lock() {
            if ring.len() >= RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(line.clone());
        }
        if let Ok(app) = self.app.lock() {
            if let Some(app) = app.as_ref() {
                let _ = app.emit("log-line", &line);
            }
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            if let Some(file) = file.as_mut() {
                let _ = file.flush();
            }
        }
    }
}

pub fn init(log_path: &Path, echo_stdout: bool) {
    if let Ok(meta) = std::fs::metadata(log_path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::remove_file(log_path);
        }
    }
    let file = OpenOptions::new().create(true).append(true).open(log_path).ok();
    let logger = Logger {
        file: Mutex::new(file),
        ring: Mutex::new(VecDeque::new()),
        app: Mutex::new(None),
        echo_stdout,
    };
    if LOGGER.set(logger).is_ok() {
        if let Some(logger) = LOGGER.get() {
            let _ = log::set_logger(logger);
            log::set_max_level(LevelFilter::Debug);
        }
    }
}

/// Start forwarding new lines to the webview.
pub fn attach_app(app: AppHandle) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut slot) = logger.app.lock() {
            *slot = Some(app);
        }
    }
}

pub fn recent(limit: usize) -> Vec<String> {
    LOGGER
        .get()
        .and_then(|l| l.ring.lock().ok())
        .map(|ring| ring.iter().rev().take(limit).rev().cloned().collect())
        .unwrap_or_default()
}
