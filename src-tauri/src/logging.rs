//! One logger for everything: a per-session file under `logs\`, an in-memory ring the UI
//! can fetch, a `log-line` event for live tailing, and optional stdout echo for CLI runs.
//!
//! Every run writes its own `launcher-YYYYMMDD-HHMMSS.log` (`cli-...` for `--cli`), so the
//! run where something broke is still on disk when a friend reports it. Lines are held in
//! memory until `open_file` (the GUI calls it once it knows it is the single instance, so a
//! double-click that only hands off to the running launcher leaves no file behind).
//! `prune` keeps the folder bounded; see `plan_prune` for the rules.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use log::{Level, LevelFilter, Log, Metadata, Record};
use tauri::{AppHandle, Emitter};

const RING_CAPACITY: usize = 2000;
/// Lines kept while no file is open yet (a GUI that never becomes the primary instance).
const PENDING_CAPACITY: usize = 5000;
/// A session file rolls over to `<stem>.2.log`, `.3.log`, ... past this size.
const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Retention for session logs and crash reports (see `plan_prune`).
const KEEP_SESSIONS: usize = 20;
const KEEP_SESSIONS_ALWAYS: usize = 5;
const SESSION_MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
const SESSIONS_MAX_TOTAL: u64 = 64 * 1024 * 1024;
const KEEP_CRASHES: usize = 10;
const KEEP_CRASHES_ALWAYS: usize = 3;
const CRASH_MAX_AGE: Duration = Duration::from_secs(90 * 24 * 3600);

/// The single file older versions appended to. It could hold whole chats (SillyTavern's
/// debug output), so it is deleted rather than kept around or bundled.
pub const LEGACY_LOG: &str = "launcher.log";

#[derive(Clone, Copy, Debug)]
pub enum SessionKind {
    Gui,
    Cli,
}

impl SessionKind {
    fn prefix(self) -> &'static str {
        match self {
            SessionKind::Gui => "launcher",
            SessionKind::Cli => "cli",
        }
    }
}

struct OpenFile {
    file: File,
    path: PathBuf,
    written: u64,
    part: u32,
}

enum Sink {
    Pending(Vec<String>),
    Open(OpenFile),
    /// The file could not be created; the ring and stdout still work.
    Unavailable,
}

struct Logger {
    dir: PathBuf,
    /// `launcher-20261102-143012`: every file of this session starts with it.
    stem: String,
    sink: Mutex<Sink>,
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
        if let Ok(mut sink) = self.sink.lock() {
            self.write_line(&mut sink, &format!("{} {line}", now.format("%Y-%m-%d")));
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
        if let Ok(mut sink) = self.sink.lock() {
            if let Sink::Open(open) = &mut *sink {
                let _ = open.file.flush();
            }
        }
    }
}

impl Logger {
    fn write_line(&self, sink: &mut Sink, line: &str) {
        match sink {
            Sink::Pending(lines) => {
                if lines.len() < PENDING_CAPACITY {
                    lines.push(line.to_string());
                }
            }
            Sink::Open(open) => {
                if writeln!(open.file, "{line}").is_ok() {
                    open.written += line.len() as u64 + 1;
                }
                if open.written > MAX_FILE_BYTES {
                    let part = open.part + 1;
                    let path = self.dir.join(format!("{}.{part}.log", self.stem));
                    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
                        *open = OpenFile {
                            file,
                            path,
                            written: 0,
                            part,
                        };
                    }
                }
            }
            Sink::Unavailable => {}
        }
    }
}

/// Sets up logging for this process. Nothing touches the disk until `open_file`.
pub fn init(logs_dir: &Path, kind: SessionKind, echo_stdout: bool) {
    let stem = format!(
        "{}-{}",
        kind.prefix(),
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let logger = Logger {
        dir: logs_dir.to_path_buf(),
        stem,
        sink: Mutex::new(Sink::Pending(Vec::new())),
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

/// Creates this session's file (writing everything logged so far), removes the legacy
/// single log, and prunes old sessions.
pub fn open_file() {
    let Some(logger) = LOGGER.get() else { return };
    let opened = {
        let Ok(mut sink) = logger.sink.lock() else { return };
        let Sink::Pending(pending) = &mut *sink else { return };
        let pending = std::mem::take(pending);
        let path = logger.dir.join(format!("{}.log", logger.stem));
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(mut file) => {
                let mut written = 0u64;
                for line in &pending {
                    if writeln!(file, "{line}").is_ok() {
                        written += line.len() as u64 + 1;
                    }
                }
                *sink = Sink::Open(OpenFile {
                    file,
                    path,
                    written,
                    part: 1,
                });
                Ok(())
            }
            Err(e) => {
                *sink = Sink::Unavailable;
                Err(format!("{}: {e}", path.display()))
            }
        }
    };
    if let Err(e) = opened {
        log::warn!("could not create the log file {e}; logging to memory only");
        return;
    }
    let legacy = logger.dir.join(LEGACY_LOG);
    if legacy.is_file() && std::fs::remove_file(&legacy).is_ok() {
        log::info!("removed the old single log file ({LEGACY_LOG}); logs are per session now");
    }
    match prune(&logger.dir, &logger.stem) {
        Ok(0) => {}
        Ok(n) => log::info!("log cleanup: removed {n} old file(s)"),
        Err(e) => log::warn!("log cleanup failed: {e}"),
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

/// `recent` for the panic hook: never blocks (the panic may have happened under the lock).
pub fn recent_nonblocking(limit: usize) -> Vec<String> {
    LOGGER
        .get()
        .and_then(|l| l.ring.try_lock().ok())
        .map(|ring| ring.iter().rev().take(limit).rev().cloned().collect())
        .unwrap_or_default()
}

/// Writes one line to the session file without going through `log` and without blocking.
pub fn write_nonblocking(message: &str) {
    let Some(logger) = LOGGER.get() else { return };
    if let Ok(mut sink) = logger.sink.try_lock() {
        let now = chrono::Local::now();
        logger.write_line(&mut sink, &format!("{} [ERROR] {message}", now.format("%Y-%m-%d %H:%M:%S")));
    }
}

/// The file this session is writing to right now, once it exists.
pub fn session_file() -> Option<PathBuf> {
    let logger = LOGGER.get()?;
    let sink = logger.sink.try_lock().ok()?;
    match &*sink {
        Sink::Open(open) => Some(open.path.clone()),
        _ => None,
    }
}

pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        logger.flush();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileKind {
    Session,
    Crash,
}

/// `launcher-*.log` / `cli-*.log` are session logs, `crash-*.txt` crash reports; anything
/// else in the folder (the session marker, files a user dropped there) is left alone.
pub fn classify(name: &str) -> Option<FileKind> {
    let lower = name.to_ascii_lowercase();
    if (lower.starts_with("launcher-") || lower.starts_with("cli-")) && lower.ends_with(".log") {
        Some(FileKind::Session)
    } else if lower.starts_with("crash-") && lower.ends_with(".txt") {
        Some(FileKind::Crash)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
pub struct LogFile {
    pub name: String,
    pub modified: SystemTime,
    pub len: u64,
}

/// Every session log and crash report in `dir`, newest first.
pub fn list(dir: &Path) -> Vec<(FileKind, LogFile)> {
    let mut files: Vec<(FileKind, LogFile)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = classify(&name)?;
            let meta = entry.metadata().ok().filter(|m| m.is_file())?;
            Some((
                kind,
                LogFile {
                    name,
                    modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    len: meta.len(),
                },
            ))
        })
        .collect();
    files.sort_by(|a, b| b.1.modified.cmp(&a.1.modified));
    files
}

/// Which files to delete. Session logs: the newest 20 stay, but anything older than 30 days
/// goes unless it is among the newest 5 (someone who has not opened the launcher in two
/// months still has their last runs), and the total stays under 64 MB. Crash reports: the
/// newest 10, nothing older than 90 days beyond the newest 3. The current session's files
/// are never touched.
pub fn plan_prune(files: &[(FileKind, LogFile)], now: SystemTime, current_stem: &str) -> Vec<String> {
    let age = |f: &LogFile| now.duration_since(f.modified).unwrap_or_default();
    let mut newest_first: Vec<&(FileKind, LogFile)> = files.iter().collect();
    newest_first.sort_by(|a, b| b.1.modified.cmp(&a.1.modified));

    let mut delete = Vec::new();
    let mut kept_sessions: Vec<&LogFile> = Vec::new();
    let (mut sessions_seen, mut crashes_seen) = (0usize, 0usize);
    for (kind, file) in newest_first {
        let current = file.name.starts_with(current_stem);
        match kind {
            FileKind::Session => {
                let index = sessions_seen;
                sessions_seen += 1;
                let keep = current
                    || (index < KEEP_SESSIONS
                        && (index < KEEP_SESSIONS_ALWAYS || age(file) <= SESSION_MAX_AGE));
                if keep {
                    kept_sessions.push(file);
                } else {
                    delete.push(file.name.clone());
                }
            }
            FileKind::Crash => {
                let index = crashes_seen;
                crashes_seen += 1;
                let keep =
                    index < KEEP_CRASHES && (index < KEEP_CRASHES_ALWAYS || age(file) <= CRASH_MAX_AGE);
                if !keep {
                    delete.push(file.name.clone());
                }
            }
        }
    }

    // Size cap: drop the oldest kept session logs (never the current one) until under.
    let mut total: u64 = kept_sessions.iter().map(|f| f.len).sum();
    for file in kept_sessions.iter().rev() {
        if total <= SESSIONS_MAX_TOTAL {
            break;
        }
        if file.name.starts_with(current_stem) {
            continue;
        }
        total -= file.len;
        delete.push(file.name.clone());
    }
    delete
}

/// Applies `plan_prune` to `dir`; returns how many files were removed.
pub fn prune(dir: &Path, current_stem: &str) -> std::io::Result<usize> {
    let files = list(dir);
    let mut removed = 0;
    for name in plan_prune(&files, SystemTime::now(), current_stem) {
        if std::fs::remove_file(dir.join(&name)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 24 * 3600;

    fn file(kind: FileKind, name: &str, days_old: u64, len: u64, now: SystemTime) -> (FileKind, LogFile) {
        (
            kind,
            LogFile {
                name: name.to_string(),
                modified: now - Duration::from_secs(days_old * DAY),
                len,
            },
        )
    }

    #[test]
    fn classifies_only_our_files() {
        assert_eq!(classify("launcher-20261102-143012.log"), Some(FileKind::Session));
        assert_eq!(classify("launcher-20261102-143012.2.log"), Some(FileKind::Session));
        assert_eq!(classify("cli-20261102-143012.log"), Some(FileKind::Session));
        assert_eq!(classify("crash-20261102-143012-panic.txt"), Some(FileKind::Crash));
        assert_eq!(classify("session.json"), None);
        assert_eq!(classify("launcher.log"), None);
        assert_eq!(classify("notes.txt"), None);
    }

    #[test]
    fn keeps_newest_twenty_sessions() {
        let now = SystemTime::now();
        // launcher-00 is the newest (a minute apart), all well inside the age limit.
        let files: Vec<_> = (0..25u64)
            .map(|i| file(FileKind::Session, &format!("launcher-{i:02}.log"), 0, 1000, now - Duration::from_secs(i * 60)))
            .collect();
        let mut delete = plan_prune(&files, now, "launcher-current");
        delete.sort();
        assert_eq!(delete, vec!["launcher-20.log", "launcher-21.log", "launcher-22.log", "launcher-23.log", "launcher-24.log"]);
    }

    #[test]
    fn old_sessions_go_but_the_newest_five_always_stay() {
        let now = SystemTime::now();
        let files: Vec<_> = (0..8)
            .map(|i| file(FileKind::Session, &format!("launcher-{i}.log"), 40 + i, 1000, now))
            .collect();
        let mut delete = plan_prune(&files, now, "launcher-current");
        delete.sort();
        assert_eq!(delete, vec!["launcher-5.log", "launcher-6.log", "launcher-7.log"]);
    }

    #[test]
    fn size_cap_drops_oldest_but_never_the_current_session() {
        let now = SystemTime::now();
        let mb = 1024 * 1024;
        let files = vec![
            file(FileKind::Session, "launcher-now.log", 0, 40 * mb, now),
            file(FileKind::Session, "launcher-b.log", 1, 20 * mb, now),
            file(FileKind::Session, "launcher-c.log", 2, 20 * mb, now),
            file(FileKind::Session, "launcher-d.log", 3, 1 * mb, now),
        ];
        let delete = plan_prune(&files, now, "launcher-now");
        // 81 MB total: d (oldest) goes first, then c; 60 MB remain.
        assert_eq!(delete, vec!["launcher-d.log", "launcher-c.log"]);
    }

    #[test]
    fn crash_reports_have_their_own_limits() {
        let now = SystemTime::now();
        let mut files: Vec<_> = (0..12)
            .map(|i| file(FileKind::Crash, &format!("crash-{i:02}.txt"), i, 500, now))
            .collect();
        files.push(file(FileKind::Session, "launcher-x.log", 0, 500, now));
        let mut delete = plan_prune(&files, now, "launcher-x");
        delete.sort();
        assert_eq!(delete, vec!["crash-10.txt", "crash-11.txt"]);

        let old: Vec<_> = (0..5)
            .map(|i| file(FileKind::Crash, &format!("crash-old{i}.txt"), 100 + i, 500, now))
            .collect();
        let mut delete = plan_prune(&old, now, "launcher-x");
        delete.sort();
        assert_eq!(delete, vec!["crash-old3.txt", "crash-old4.txt"]);
    }
}
