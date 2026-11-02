//! Evidence for "the launcher just closed" reports, written to `logs\crash-*.txt`:
//!
//! - a panic hook (any Rust panic, on any thread), with the message, source location,
//!   thread and the last log lines;
//! - a session marker (`logs\session.json`) the GUI writes on start and removes when it
//!   quits, when Windows signs out or shuts down (`WM_ENDSESSION`, see `gui.rs`) or when it
//!   updates itself. If the marker is still there on the next start, the previous run died
//!   without any of those: a native crash, Task Manager, an out-of-memory kill. A real
//!   restart or power loss is told apart by the boot time and only noted in the log.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::{logging, selfupdate, sysinfo};

const MARKER: &str = "session.json";
/// Log lines copied into a report.
const TAIL_LINES: usize = 200;

static LOGS_DIR: OnceLock<PathBuf> = OnceLock::new();
static SESSION_OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Deserialize, Debug)]
struct Marker {
    pid: u32,
    /// RFC 3339, local time.
    started: String,
    version: String,
    /// File name of that session's log.
    log: Option<String>,
}

/// Writes a crash report for every panic, then lets the default hook print as usual.
pub fn install_panic_hook(logs_dir: &Path) {
    let _ = LOGS_DIR.set(logs_dir.to_path_buf());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "(no message)".to_string()
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let thread = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_string();
        logging::write_nonblocking(&format!("PANIC in thread '{thread}' at {location}: {message}"));

        // Release builds are stripped and the PDB is not shipped, so a backtrace would be
        // bare addresses; the location above is what pins it down.
        let backtrace = cfg!(debug_assertions)
            .then(|| std::backtrace::Backtrace::force_capture().to_string());
        let mut fields = vec![
            ("thread", thread),
            ("location", location),
            ("message", message),
        ];
        if let Some(bt) = backtrace {
            fields.push(("backtrace", format!("\n{bt}")));
        }
        let body = render(
            "panic",
            &fields,
            logging::session_file().as_deref(),
            &logging::recent_nonblocking(TAIL_LINES),
        );
        if let Some(dir) = LOGS_DIR.get() {
            let _ = write_report(dir, "panic", &body);
        }
        previous(info);
    }));
}

/// GUI start, once this process is the single instance: reports a previous session that
/// died, then marks this one as running.
pub fn begin_session(logs_dir: &Path) {
    let _ = LOGS_DIR.set(logs_dir.to_path_buf());
    let marker_path = logs_dir.join(MARKER);
    if let Some(previous) = std::fs::read_to_string(&marker_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Marker>(&text).ok())
    {
        report_previous(logs_dir, &previous);
    }
    let marker = Marker {
        pid: std::process::id(),
        started: Local::now().to_rfc3339(),
        version: selfupdate::CURRENT_VERSION.to_string(),
        log: logging::session_file()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
    };
    match serde_json::to_string_pretty(&marker) {
        Ok(json) => {
            if std::fs::write(&marker_path, json).is_ok() {
                SESSION_OPEN.store(true, Ordering::SeqCst);
            }
        }
        Err(e) => log::warn!("could not write the session marker: {e}"),
    }
}

/// A clean end of this session: quit, self-update, or Windows signing out / shutting down.
/// Safe to call more than once and from processes that never began a session.
pub fn end_session(reason: &str) {
    if !SESSION_OPEN.swap(false, Ordering::SeqCst) {
        return;
    }
    log::info!("session ended: {reason}");
    logging::flush();
    if let Some(dir) = LOGS_DIR.get() {
        let _ = std::fs::remove_file(dir.join(MARKER));
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Windows booted after that session started: a restart or power loss, not our crash.
    Rebooted,
    /// Windows kept running and the launcher vanished without quitting.
    Unclean,
}

fn classify(started: SystemTime, boot: Option<SystemTime>) -> Verdict {
    match boot {
        Some(boot) if boot > started => Verdict::Rebooted,
        _ => Verdict::Unclean,
    }
}

fn report_previous(logs_dir: &Path, previous: &Marker) {
    let started = DateTime::parse_from_rfc3339(&previous.started).ok();
    let started_text = started
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| previous.started.clone());
    let verdict = match started {
        Some(t) => classify(SystemTime::from(t), sysinfo::boot_time()),
        None => Verdict::Unclean,
    };
    match verdict {
        Verdict::Rebooted => log::info!(
            "the previous session (started {started_text}) ended with a Windows restart or power loss"
        ),
        Verdict::Unclean => {
            let tail = previous
                .log
                .as_deref()
                .map(|name| tail_of(&logs_dir.join(name), TAIL_LINES))
                .unwrap_or_default();
            let body = render(
                "unclean exit",
                &[
                    (
                        "what",
                        "the previous launcher session ended without quitting (crash, closed from Task Manager, or killed)".to_string(),
                    ),
                    ("started", started_text.clone()),
                    ("pid", previous.pid.to_string()),
                    ("version", previous.version.clone()),
                ],
                previous.log.as_deref().map(Path::new),
                &tail,
            );
            match write_report(logs_dir, "unclean-exit", &body) {
                Ok(path) => log::warn!(
                    "the previous session (started {started_text}, launcher {}) ended without quitting; details in {}",
                    previous.version,
                    path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ),
                Err(e) => log::warn!("the previous session ended without quitting; could not write a report: {e}"),
            }
        }
    }
}

/// Last `n` lines of a log file (whole file read; session logs are capped at 10 MB).
fn tail_of(path: &Path, n: usize) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return vec![format!("(log {} not found)", path.display())];
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..]
        .iter()
        .map(|l| l.to_string())
        .collect()
}

fn render(kind: &str, fields: &[(&str, String)], log: Option<&Path>, tail: &[String]) -> String {
    let mut out = vec![
        format!("SillyTavern Launcher crash report ({kind})"),
        format!("time:      {}", Local::now().format("%Y-%m-%d %H:%M:%S %:z")),
        format!("launcher:  {}", selfupdate::CURRENT_VERSION),
        format!("windows:   {}", sysinfo::windows_version()),
    ];
    if let Some(name) = log.and_then(Path::file_name) {
        out.push(format!("log file:  {}", name.to_string_lossy()));
    }
    for (key, value) in fields {
        out.push(format!("{:<10} {value}", format!("{key}:")));
    }
    out.push(String::new());
    out.push(format!("--- last {} log lines ---", tail.len()));
    out.extend(tail.iter().cloned());
    out.push(String::new());
    out.join("\n")
}

fn write_report(dir: &Path, kind: &str, body: &str) -> std::io::Result<PathBuf> {
    let path = dir.join(format!(
        "crash-{}-{kind}.txt",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reboot_after_session_start_is_not_a_crash() {
        let started = SystemTime::now() - Duration::from_secs(3600);
        let boot = SystemTime::now() - Duration::from_secs(600);
        assert_eq!(classify(started, Some(boot)), Verdict::Rebooted);
    }

    #[test]
    fn session_after_boot_that_vanished_is_unclean() {
        let boot = SystemTime::now() - Duration::from_secs(86_400);
        let started = SystemTime::now() - Duration::from_secs(3600);
        assert_eq!(classify(started, Some(boot)), Verdict::Unclean);
        assert_eq!(classify(started, None), Verdict::Unclean);
    }

    #[test]
    fn report_has_header_fields_and_tail() {
        let body = render(
            "panic",
            &[("location", "src/x.rs:1:2".to_string())],
            Some(Path::new("logs/launcher-1.log")),
            &["a".to_string(), "b".to_string()],
        );
        assert!(body.starts_with("SillyTavern Launcher crash report (panic)\n"));
        assert!(body.contains("\nlocation:  src/x.rs:1:2\n"));
        assert!(body.contains("\nlog file:  launcher-1.log\n"));
        assert!(body.contains("--- last 2 log lines ---\na\nb\n"));
    }
}
