//! Runs `node server.js` without a console, tracks readiness, reads its log for
//! phone connections, and tears it down.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::net;
use crate::paths::Paths;
use crate::process::{self, JobObject};
use crate::state::Settings;

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

/// A client SillyTavern reported in its log ("New connection from ..." / "Blocked ...").
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionEvent {
    pub ip: String,
    pub user_agent: String,
    pub blocked: bool,
    /// Local wall-clock time, "15:42".
    pub at: String,
    /// "iPhone", "iPad", "Android phone", "another device"
    pub device: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Connections {
    /// Most recent accepted connection from something other than this PC.
    pub last_remote: Option<ConnectionEvent>,
    /// Most recent connection the whitelist rejected.
    pub last_blocked: Option<ConnectionEvent>,
}

pub const READY_TIMEOUT: Duration = Duration::from_secs(180);
const TAIL_LINES: usize = 12;

type Notifier = Box<dyn Fn(ServerStatus) + Send + Sync>;
type ConnectionNotifier = Box<dyn Fn(ConnectionEvent) + Send + Sync>;

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
    connections: Mutex<Connections>,
    connection_notifier: Mutex<Option<ConnectionNotifier>>,
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
            connections: Mutex::new(Connections::default()),
            connection_notifier: Mutex::new(None),
        }
    }

    pub fn set_notifier(&self, f: Notifier) {
        *self.notifier.lock().unwrap() = Some(f);
    }

    pub fn set_connection_notifier(&self, f: ConnectionNotifier) {
        *self.connection_notifier.lock().unwrap() = Some(f);
    }

    pub fn status(&self) -> ServerStatus {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn connections(&self) -> Connections {
        self.connections.lock().unwrap().clone()
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

    fn record_connection(&self, event: ConnectionEvent) {
        {
            let mut c = self.connections.lock().unwrap();
            if event.blocked {
                c.last_blocked = Some(event.clone());
            } else {
                c.last_remote = Some(event.clone());
            }
        }
        if let Some(n) = self.connection_notifier.lock().unwrap().as_ref() {
            n(event);
        }
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
                        "SillyTavern was already running (started outside the launcher, e.g. with Start.bat). Phone access then depends on that copy's own settings."
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
            // SillyTavern ships with `logging.minLogLevel: 0` (DEBUG), and at DEBUG it prints
            // the whole request body of every generation: character cards, chat history,
            // everything. Our log files (and the bundles friends send) must not carry that.
            // INFO keeps what we parse: "New connection from" is info, "Blocked connection"
            // is warn. Environment variables override config.yaml in SillyTavern's getConfigValue.
            .env("SILLYTAVERN_LOGGING_MINLOGLEVEL", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        log::info!(
            "starting SillyTavern on port {} (lan={}) from {}",
            settings.port,
            settings.lan_enabled,
            st_dir.display()
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
            spawn_tail_logger(out, "ST", Arc::clone(&tail), Arc::clone(self));
        }
        if let Some(err) = stderr {
            spawn_tail_logger(err, "ST!", Arc::clone(&tail), Arc::clone(self));
        }

        let me = Arc::clone(self);
        let port = settings.port;
        std::thread::spawn(move || me.watch_startup(generation, port, url, tail));
        Ok(())
    }

    /// Closes a SillyTavern that was started outside the launcher (only ever a node.exe
    /// holding the port) and starts ours in its place.
    pub fn take_over(self: &Arc<Self>, paths: &Paths, settings: &Settings) -> Result<()> {
        let listeners = net::listeners_on_port(settings.port);
        let mut killed = 0;
        for l in &listeners {
            match l.process.as_deref() {
                Some(p) if p.eq_ignore_ascii_case("node.exe") => {
                    log::info!("closing node.exe pid {} that holds port {}", l.pid, settings.port);
                    net::kill_pid(l.pid)?;
                    killed += 1;
                }
                Some(p) => bail!(
                    "Port {} is used by {p}, which is not SillyTavern. Close that program and try again.",
                    settings.port
                ),
                None => bail!(
                    "Port {} is used by another program (pid {}). Close it and try again.",
                    settings.port,
                    l.pid
                ),
            }
        }
        if killed > 0 {
            let deadline = Instant::now() + Duration::from_secs(8);
            while net::port_open(settings.port) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        self.set_status(ServerStatus {
            state: ServerState::Stopped,
            url: local_url(settings.port),
            message: None,
            pid: None,
            external: false,
        });
        self.start(paths, settings)
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

/// Parses SillyTavern's "New connection from 192.168.1.50; User Agent: ..." and
/// "Blocked connection from ..." lines. Returns None for this PC's own browser.
pub fn parse_connection_line(line: &str) -> Option<ConnectionEvent> {
    let (blocked, rest) = if let Some(r) = line.split_once("New connection from ") {
        (false, r.1)
    } else if let Some(r) = line.split_once("Blocked connection from ") {
        (true, r.1)
    } else {
        return None;
    };
    let (ip_part, ua) = rest.split_once("; User Agent:").unwrap_or((rest, ""));
    let ip = ip_part
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    if ip.is_empty() || ip == "127.0.0.1" || ip == "::1" || ip.starts_with("::ffff:127.") {
        return None;
    }
    let ua = ua.trim().to_string();
    let lower = ua.to_ascii_lowercase();
    let device = if lower.contains("iphone") {
        "iPhone"
    } else if lower.contains("ipad") {
        "iPad"
    } else if lower.contains("android") {
        "Android phone"
    } else if lower.contains("mobile") {
        "a phone"
    } else {
        "another device"
    };
    Some(ConnectionEvent {
        ip,
        user_agent: ua,
        blocked,
        at: chrono::Local::now().format("%H:%M").to_string(),
        device: device.to_string(),
    })
}

fn spawn_tail_logger<R: Read + Send + 'static>(
    reader: R,
    label: &'static str,
    tail: Arc<Mutex<VecDeque<String>>>,
    manager: Arc<ServerManager>,
) {
    std::thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            log::info!(target: "sillytavern", "[{label}] {line}");
            if let Some(event) = parse_connection_line(&line) {
                manager.record_connection(event);
            }
            let mut tail = tail.lock().unwrap();
            if tail.len() >= TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::parse_connection_line;

    #[test]
    fn parses_phone_connections_and_ignores_localhost() {
        let ev = parse_connection_line(
            "New connection from 192.168.1.50; User Agent: Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X)",
        )
        .unwrap();
        assert_eq!(ev.ip, "192.168.1.50");
        assert_eq!(ev.device, "iPhone");
        assert!(!ev.blocked);

        let blocked = parse_connection_line(
            "Blocked connection from 192.168.1.77 (forwarded from 10.0.0.2); User Agent: Mozilla/5.0 (Linux; Android 16)",
        )
        .unwrap();
        assert_eq!(blocked.ip, "192.168.1.77");
        assert_eq!(blocked.device, "Android phone");
        assert!(blocked.blocked);

        assert!(parse_connection_line("New connection from 127.0.0.1; User Agent: curl/8.19.0").is_none());
        assert!(parse_connection_line("SillyTavern is listening on IPv4: 0.0.0.0:8000").is_none());
    }
}
