use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Mirrors the UI's step states; `Pending`/`Error` exist for protocol completeness.
#[allow(dead_code)]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Skipped,
    Error,
}

/// Progress events for a long task (install or update). The UI mirrors these 1:1.
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TaskEvent {
    Step {
        task: String,
        step: String,
        status: StepStatus,
        detail: String,
    },
    Progress {
        task: String,
        step: String,
        done: u64,
        total: Option<u64>,
        unit: String,
    },
    Finished {
        task: String,
        ok: bool,
        error: Option<String>,
    },
}

/// Sink for task progress. Object-safe so the installer can take `&dyn Reporter`.
pub trait Reporter: Send + Sync {
    fn task(&self) -> &str;
    fn emit(&self, event: TaskEvent);

    fn step(&self, step: &str, status: StepStatus, detail: &str) {
        log::info!("[{step}] {status:?}: {detail}");
        self.emit(TaskEvent::Step {
            task: self.task().to_string(),
            step: step.to_string(),
            status,
            detail: detail.to_string(),
        });
    }

    fn progress(&self, step: &str, done: u64, total: Option<u64>, unit: &str) {
        self.emit(TaskEvent::Progress {
            task: self.task().to_string(),
            step: step.to_string(),
            done,
            total,
            unit: unit.to_string(),
        });
    }

    fn finished(&self, ok: bool, error: Option<String>) {
        self.emit(TaskEvent::Finished {
            task: self.task().to_string(),
            ok,
            error,
        });
    }
}

/// Forwards events to the webview as `task-event`.
pub struct TauriReporter {
    pub app: AppHandle,
    pub task: String,
}

impl Reporter for TauriReporter {
    fn task(&self) -> &str {
        &self.task
    }

    fn emit(&self, event: TaskEvent) {
        if let Err(e) = self.app.emit("task-event", &event) {
            log::warn!("could not emit task event: {e}");
        }
    }
}

/// Prints events to stdout for `--cli` runs.
pub struct CliReporter {
    task: String,
    last_percent: Mutex<Option<u64>>,
}

impl CliReporter {
    pub fn new(task: &str) -> Self {
        Self {
            task: task.to_string(),
            last_percent: Mutex::new(None),
        }
    }
}

impl Reporter for CliReporter {
    fn task(&self) -> &str {
        &self.task
    }

    fn emit(&self, event: TaskEvent) {
        match event {
            TaskEvent::Step { .. } => {} // already logged by `step()` via the logger's stdout echo
            TaskEvent::Progress {
                done, total, unit, ..
            } => {
                let Some(total) = total.filter(|t| *t > 0) else {
                    return;
                };
                let percent = done * 100 / total;
                let mut last = self.last_percent.lock().unwrap();
                if last.map_or(true, |p| percent >= p + 10 || percent == 100 && p != 100) {
                    *last = Some(percent);
                    if unit == "bytes" {
                        println!(
                            "    {percent:>3}%  {} / {}",
                            crate::download::human_bytes(done),
                            crate::download::human_bytes(total)
                        );
                    } else {
                        println!("    {percent:>3}%  {done} / {total} {unit}");
                    }
                }
            }
            TaskEvent::Finished { ok, error, .. } => {
                if ok {
                    println!("Finished.");
                } else {
                    println!("FAILED: {}", error.unwrap_or_default());
                }
            }
        }
    }
}
