//! "Save logs to Desktop": one zip a non-technical user can send to whoever helps them.
//!
//! Contents: `summary.txt` (versions, Windows, install, settings, config flags, addresses,
//! the phone-access checks), the most recent session logs and every crash report. The
//! user's profile folder is replaced by `%USERPROFILE%` everywhere and other people's
//! `Users\<name>` folders by `Users\<user>`. Never included: config.yaml itself (it can
//! hold basic-auth credentials), secrets.json, or anything from SillyTavern's data folder.
//! Chats do not reach the logs in the first place (the server runs at INFO, see server.rs).

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use zip::write::SimpleFileOptions;

use crate::logging::{self, FileKind};
use crate::paths::Paths;
use crate::server::ServerStatus;
use crate::state::{self, LauncherState};
use crate::{config, diagnose, git, net, selfupdate, shortcut, sillytavern, sysinfo};

/// Session logs per bundle, newest first, within this many bytes before compression.
const BUNDLE_SESSIONS: usize = 6;
const BUNDLE_MAX_LOG_BYTES: u64 = 30 * 1024 * 1024;

/// Writes the zip to `dest_dir` (the Desktop when `None`) and returns its path.
pub fn create_bundle(paths: &Paths, server: &ServerStatus, dest_dir: Option<PathBuf>) -> Result<PathBuf> {
    logging::flush();
    let dest_dir = dest_dir
        .or_else(dirs::desktop_dir)
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| paths.home.clone());
    let zip_path = dest_dir.join(format!(
        "SillyTavern-Launcher-logs-{}.zip",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    let profile = user_profile();
    let clean = |text: &str| redact(text, profile.as_deref());

    let file = std::fs::File::create(&zip_path)
        .with_context(|| format!("could not create {}", zip_path.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("summary.txt", options)?;
    zip.write_all(clean(&summary(paths, server)).as_bytes())?;

    let logs_dir = paths.logs();
    let mut sessions = 0usize;
    let mut log_bytes = 0u64;
    let mut included = Vec::new();
    for (kind, file) in logging::list(&logs_dir) {
        let folder = match kind {
            FileKind::Crash => "crashes",
            FileKind::Session => {
                if sessions >= BUNDLE_SESSIONS || log_bytes + file.len > BUNDLE_MAX_LOG_BYTES {
                    continue;
                }
                sessions += 1;
                log_bytes += file.len;
                "logs"
            }
        };
        let Ok(bytes) = std::fs::read(logs_dir.join(&file.name)) else {
            continue;
        };
        zip.start_file(format!("{folder}/{}", file.name), options)?;
        zip.write_all(clean(&String::from_utf8_lossy(&bytes)).as_bytes())?;
        included.push(format!("{folder}/{}", file.name));
    }
    zip.finish().context("could not finish the zip file")?;
    log::info!(
        "saved a log bundle to {} ({} file(s): {})",
        zip_path.display(),
        included.len() + 1,
        included.join(", ")
    );
    Ok(zip_path)
}

/// Opens Explorer with the file selected, so it can be dragged straight into a chat.
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    // explorer.exe wants `/select,"path"` verbatim; Rust's own quoting of a single argument
    // would wrap the whole thing and Explorer then opens Documents instead.
    let spawned = std::process::Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
    if let Err(e) = spawned {
        log::warn!("could not open Explorer: {e}");
    }
}

fn user_profile() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .or_else(|| dirs::home_dir().map(|p| p.display().to_string()))
        .map(|p| p.trim_end_matches(['\\', '/']).to_string())
        .filter(|p| p.len() > 3)
}

/// Plain-text facts about this PC and the install, in the order someone debugging wants them.
fn summary(paths: &Paths, server: &ServerStatus) -> String {
    let state = LauncherState::load(paths);
    let settings = &state.settings;
    let st_dir = paths.st_dir();
    let is_git = git::is_repo(&st_dir);
    let flags = config::read_flags(&st_dir);
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let opt = |v: Option<String>| v.unwrap_or_else(|| "(unknown)".to_string());
    let opt_bool = |v: Option<bool>| v.map(|b| b.to_string()).unwrap_or_else(|| "(not set)".to_string());

    let mut out = vec![
        "SillyTavern Launcher support bundle".to_string(),
        format!("created:       {}", Local::now().format("%Y-%m-%d %H:%M:%S %:z")),
        format!("launcher:      {} ({exe})", selfupdate::CURRENT_VERSION),
        format!("windows:       {}", sysinfo::windows_version()),
        format!("webview2:      {}", sysinfo::webview_version()),
        format!("home:          {}", paths.home.display()),
        String::new(),
        format!("sillytavern:   {}", st_dir.display()),
        format!(
            "  mode:        {:?}{}",
            state.mode,
            if is_git {
                format!(", git ({})", git::branch(&st_dir).unwrap_or_else(|| "detached".to_string()))
            } else {
                String::new()
            }
        ),
        format!("  version:     {}", opt(sillytavern::version_in(&st_dir).or(state.st_version.clone()))),
        format!("  installed:   {}", state::is_installed(paths)),
        format!("node:          {}", opt(state.node_version.clone())),
        format!(
            "settings:      lan={} port={} open_browser={} check_updates={} keep_awake={} start_with_windows={}",
            settings.lan_enabled,
            settings.port,
            settings.open_browser_on_start,
            settings.check_updates_on_start,
            settings.keep_awake,
            shortcut::start_with_windows_enabled()
        ),
        format!(
            "server:        {:?} at {}{}{}{}",
            server.state,
            server.url,
            server.pid.map(|p| format!(", pid {p}")).unwrap_or_default(),
            if server.external { ", started outside the launcher" } else { "" },
            server.message.as_deref().map(|m| format!(" -- {m}")).unwrap_or_default()
        ),
        String::new(),
        format!("config.yaml:   exists={}", flags.exists),
        format!(
            "  listen={} whitelistMode={} basicAuth={} userAccounts={} port={}",
            opt_bool(flags.listen),
            opt_bool(flags.whitelist_mode),
            opt_bool(flags.basic_auth),
            opt_bool(flags.user_accounts),
            flags.port.map(|p| p.to_string()).unwrap_or_else(|| "(not set)".to_string())
        ),
        format!("  whitelist:   {}", flags.whitelist.join(", ")),
        String::new(),
        "addresses:".to_string(),
    ];
    for a in net::addresses() {
        out.push(format!(
            "  {:<16} {}{}{}",
            a.ip,
            a.adapter,
            if a.default_route { " [default route]" } else { "" },
            if a.virtual_adapter { " [virtual/VPN]" } else { "" }
        ));
    }
    let listeners = net::listeners_on_port(settings.port);
    out.push(format!("listening on port {}:", settings.port));
    if listeners.is_empty() {
        out.push("  (nothing)".to_string());
    }
    for l in listeners {
        out.push(format!(
            "  {} pid {} {}",
            l.address,
            l.pid,
            l.process.unwrap_or_else(|| "?".to_string())
        ));
    }

    let diagnosis = diagnose::run(paths, settings, server);
    out.push(String::new());
    out.push(format!("phone-access checks: {}", diagnosis.summary));
    for c in &diagnosis.checks {
        let level = format!("{:?}", c.level).to_ascii_lowercase();
        let detail = if c.detail.is_empty() {
            String::new()
        } else {
            format!(" -- {}", c.detail)
        };
        out.push(format!("  [{level:<4}] {}{detail}", c.title));
    }
    out.push(String::new());
    out.join("\n")
}

/// Hides who the user is without hiding what went wrong: their profile path becomes
/// `%USERPROFILE%`, any other `\Users\<name>\` segment becomes `\Users\<user>\`, and the
/// one secret SillyTavern prints (an account reset code) is blanked.
pub fn redact(text: &str, profile: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(profile) = profile {
        for variant in [profile.to_string(), profile.replace('\\', "/")] {
            out = replace_ignore_ascii_case(&out, &variant, "%USERPROFILE%");
        }
    }
    out = mask_user_segments(&out, '\\');
    out = mask_user_segments(&out, '/');
    out.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            match lower.find("account reset code is") {
                Some(i) => format!("{}account reset code is: [redacted]", &line[..i]),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn replace_ignore_ascii_case(haystack: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let hay = haystack.as_bytes();
    let pat = needle.as_bytes();
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    let mut i = 0;
    while i + pat.len() <= hay.len() {
        if hay[i..i + pat.len()].eq_ignore_ascii_case(pat)
            && haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + pat.len())
        {
            out.push_str(&haystack[last..i]);
            out.push_str(with);
            i += pat.len();
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&haystack[last..]);
    out
}

/// `X:\Users\Someone\...` -> `X:\Users\<user>\...`. Only a segment that is followed by
/// another separator on the same line is replaced, so a bare `Users\` never eats a line.
fn mask_user_segments(text: &str, sep: char) -> String {
    let marker = format!("{sep}users{sep}");
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut search = 0;
    while let Some(found) = lower[search..].find(&marker) {
        let start = search + found + marker.len();
        let rest = &text[start..];
        let end = rest
            .find(|c: char| c == sep || c == '\n' || c == '"' || c == '\'')
            .filter(|&e| e > 0 && rest[e..].starts_with(sep));
        match end {
            Some(e) => {
                let segment = &rest[..e];
                let keep = ["public", "default", "all users", "<user>"]
                    .iter()
                    .any(|k| segment.eq_ignore_ascii_case(k));
                if !keep {
                    out.push_str(&text[last..start]);
                    out.push_str("<user>");
                    last = start + e;
                }
                search = start + e;
            }
            None => search = start,
        }
    }
    out.push_str(&text[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_path_becomes_userprofile_in_both_slash_styles_and_any_case() {
        let text = "home: C:\\Users\\Jane Doe\\AppData\\Local\\SillyTavern\nexe c:/users/jane doe/Desktop/x.exe";
        let out = redact(text, Some("C:\\Users\\Jane Doe"));
        assert_eq!(
            out,
            "home: %USERPROFILE%\\AppData\\Local\\SillyTavern\nexe %USERPROFILE%/Desktop/x.exe"
        );
    }

    #[test]
    fn other_user_folders_are_masked_but_public_is_kept() {
        let text = "D:\\Users\\bob\\SillyTavern\\server.js and C:\\Users\\Public\\x";
        assert_eq!(
            redact(text, None),
            "D:\\Users\\<user>\\SillyTavern\\server.js and C:\\Users\\Public\\x"
        );
    }

    #[test]
    fn a_users_folder_without_a_following_separator_is_left_alone() {
        let text = "path C:\\Users\\someone";
        assert_eq!(redact(text, None), text);
    }

    #[test]
    fn reset_codes_are_blanked() {
        let text = "12:00:00 Jane, your account reset code is: 123456\nnext line";
        assert_eq!(
            redact(text, None),
            "12:00:00 Jane, your account reset code is: [redacted]\nnext line"
        );
    }

    #[test]
    fn non_ascii_text_survives() {
        let text = "C:\\Users\\Zoë\\x ünïcode 日本";
        let out = redact(text, Some("C:\\Users\\Zoë"));
        assert_eq!(out, "%USERPROFILE%\\x ünïcode 日本");
    }
}
