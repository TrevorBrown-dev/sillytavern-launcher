//! Runs `node server.js` without a console, tracks readiness, and tears it down.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::paths::Paths;
use crate::process::{self, JobObject};
use crate::state::Settings;
use crate::net;

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServerState {
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub state: ServerState,
    pub url: String,
    pub message: Option<String>,
    pub pid: Option<u32>,
    /// Something we did not start is already serving SillyTavern on the port.
    pub external: bool,
}

pub const READY_TIMEOUT: Duration = Duration::from_secs(180);
const TAIL_LINES: usize = 12;

type Notifier = Box<dyn Fn(ServerStatus) + Send + Sync>;

struct Inner {
    child: Option<Child>,
    status: ServerStatus,
    /// Bumped on every start/stop so stale watcher threads know to bow out.
    generation: u64,
}

pub struct ServerManager {
    inner: Mutex<Inner>,
    job: Option<Arc<JobObject>>,
    notifier: Mutex<Option<Notifier>>,
}

pub fn local_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

impl ServerManager {
    pub fn new(job: Option<Arc<JobObject>>, port: u16) -> Self {
        Self {
            inner: Mutex::new(Inner {
                child: None,
                status: ServerStatus {
                    state: ServerState::Stopped,
                    url: local_url(port),
                    message: None,
                    pid: None,
                    external: false,
                },
                generation: 0,
            }),
            job,
            notifier: Mutex::new(None),
        }
    }

    pub fn set_notifier(&self, f: Notifier) {
        *self.notifier.lock().unwrap() = Some(f);
    }

    pub fn status(&self) -> ServerStatus {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn is_up(&self) -> bool {
        matches!(
            self.status().state,
            ServerState::Running | ServerState::Starting
        )
    }

    fn set_status(&self, status: ServerStatus) {
        self.inner.lock().unwrap().status = status.clone();
        if let Some(n) = self.notifier.lock().unwrap().as_ref() {
            n(status);
        }
    }

    pub fn set_error(&self, message: String) {
        let url = self.status().url;
        self.set_status(ServerStatus {
            state: ServerState::Error,
            url,
            message: Some(message),
            pid: None,
            external: false,
        });
    }

    /// Starts SillyTavern unless it is already up. Returns as soon as the process is
    /// spawned; `Running` arrives through the notifier once the port answers.
    pub fn start(self: &Arc<Self>, paths: &Paths, settings: &Settings) -> Result<()> {
        let url = local_url(settings.port);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(child) = inner.child.as_mut() {
                match child.try_wait() {
                    Ok(None) => return Ok(()), // still running (or starting)
                    _ => inner.child = None,
                }
            }
        }

        if net::port_open(settings.port) {
            if net::looks_like_sillytavern(settings.port) {
                log::info!("SillyTavern is already answering on port {}", settings.port);
                self.set_status(ServerStatus {
                    state: ServerState::Running,
                    url,
                    message: Some(
                        "SillyTavern was already running (started outside the launcher, e.g. with Start.bat). Close that one if you want the launcher to manage it."
                            .to_string(),
                    ),
                    pid: None,
                    external: true,
                });
                return Ok(());
            }
            bail!(
                "Port {} is already used by another program. Close it and try again.",
                settings.port
            );
        }

        let st_dir = paths.st_dir();
        let mut cmd = process::quiet_command(paths.node_exe());
        cmd.arg("server.js")
            .arg(format!("--listen={}", settings.lan_enabled))
            .arg("--browserLaunchEnabled=false")
            .arg("--port")
            .arg(settings.port.to_string())
            .current_dir(&st_dir)
            .env("PATH", process::prefixed_path(&paths.node_dir()))
            .env("NODE_ENV", "production")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        log::info!(
            "starting SillyTavern on port {} (lan={})",
            settings.port,
            settings.lan_enabled
        );
        let mut child = cmd
            .spawn()
            .context("could not start SillyTavern (node.exe failed to launch)")?;
        process::attach_to_job(self.job.as_deref(), &child, "SillyTavern");
        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let generation = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.child = Some(child);
            inner.generation
        };
        self.set_status(ServerStatus {
            state: ServerState::Starting,
            url: url.clone(),
            message: None,
            pid: Some(pid),
            external: false,
        });

        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(TAIL_LINES)));
        if let Some(out) = stdout {
            spawn_tail_logger(out, "ST", Arc::clone(&tail));
        }
        if let Some(err) = stderr {
            spawn_tail_logger(err, "ST!", Arc::clone(&tail));
        }

        let me = Arc::clone(self);
        let port = settings.port;
        std::thread::spawn(move || me.watch_startup(generation, port, url, tail));
        Ok(())
    }

    fn watch_startup(
        &self,
        generation: u64,
        port: u16,
        url: String,
        tail: Arc<Mutex<VecDeque<String>>>,
    ) {
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let exited = {
                let mut inner = self.inner.lock().unwrap();
                if inner.generation != generation {
                    return; // superseded by a stop/restart
                }
                match inner.child.as_mut().map(|c| c.try_wait()) {
                    Some(Ok(Some(status))) => Some(status),
                    Some(_) => None,
                    None => return,
                }
            };
            if let Some(status) = exited {
                let recent = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
                self.inner.lock().unwrap().child = None;
                log::error!("SillyTavern exited during startup: {status}");
                self.set_status(ServerStatus {
                    state: ServerState::Error,
                    url,
                    message: Some(format!(
                        "SillyTavern stopped unexpectedly ({status}).\n{recent}"
                    )),
                    pid: None,
                    external: false,
                });
                return;
            }
            if net::port_open(port) {
                log::info!("SillyTavern is ready at {url}");
                // Lock released here; set_status takes the same mutex (never nest them).
                let pid = self.inner.lock().unwrap().child.as_ref().map(|c| c.id());
                self.set_status(ServerStatus {
                    state: ServerState::Running,
                    url,
                    message: None,
                    pid,
                    external: false,
                });
                self.watch_exit(generation);
                return;
            }
            if started.elapsed() > READY_TIMEOUT {
                log::error!("SillyTavern did not open its port within {READY_TIMEOUT:?}");
                let _ = self.stop();
                self.set_error("SillyTavern did not start within 3 minutes. Check the log.".to_string());
                return;
            }
        }
    }

    /// After a successful start, keep watching so a crash flips the status.
    fn watch_exit(&self, generation: u64) {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let mut inner = self.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            let Some(child) = inner.child.as_mut() else {
                return;
            };
            if let Ok(Some(status)) = child.try_wait() {
                inner.child = None;
                let url = inner.status.url.clone();
                drop(inner);
                log::error!("SillyTavern exited: {status}");
                self.set_status(ServerStatus {
                    state: ServerState::Error,
                    url,
                    message: Some(format!("SillyTavern stopped unexpectedly ({status}). Check the log.")),
                    pid: None,
                    external: false,
                });
                return;
            }
        }
    }

    pub fn stop(&self) -> Result<()> {
        let child = {
            let mut inner = self.inner.lock().unwrap();
            inner.generation += 1;
            inner.child.take()
        };
        let url = self.status().url;
        if let Some(mut child) = child {
            log::info!("stopping SillyTavern (pid {})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
        self.set_status(ServerStatus {
            state: ServerState::Stopped,
            url,
            message: None,
            pid: None,
            external: false,
        });
        Ok(())
    }
}

fn spawn_tail_logger<R: Read + Send + 'static>(
    reader: R,
    label: &'static str,
    tail: Arc<Mutex<VecDeque<String>>>,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            log::info!(target: "sillytavern", "[{label}] {line}");
            let mut tail = tail.lock().unwrap();
            if tail.len() >= TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
}
