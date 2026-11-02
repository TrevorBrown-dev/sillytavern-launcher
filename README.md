# SillyTavern Launcher

**Just want to install SillyTavern?** Go to **https://trevorbrown.dev/gh/sillytavern-launcher** and click the big button. The rest of this page is for people building the launcher.

A single Windows `.exe` that installs SillyTavern end to end for someone who has never opened a terminal, then stays around as the thing they double-click to run it. Built for friends coming over from JanitorAI who mostly want to use it from their phone.

## What it does

First run (the installer):

1. Downloads a portable Node.js (the newest 24.x) from nodejs.org, verifies the SHA-256 against `SHASUMS256.txt`, unpacks it. Nothing is added to the system PATH and no admin rights are needed.
2. Downloads the latest SillyTavern release zip from GitHub (no Git involved) and unpacks it.
3. Runs `npm ci --omit=dev --ignore-scripts` with the bundled Node, the same flags SillyTavern's own `Start.bat` uses.
4. Writes `config.yaml` from SillyTavern's defaults with `listen: true`, whitelist mode on, and the private IPv4 ranges (`10/8`, `172.16/12`, `192.168/16`) whitelisted, so phones on the home Wi-Fi can connect without typing IPs anywhere. SillyTavern's own browser auto-launch is turned off because the launcher handles that.
5. Copies itself into the install folder and creates Desktop + Start Menu shortcuts pointing at that copy (so deleting the original download does not break anything).
6. Adds an inbound Windows Firewall rule for the bundled `node.exe`, limited to `LocalSubnet`. This is the one step that needs admin, so it triggers a single UAC prompt. If the user declines, Windows will show its own "allow Node.js" prompt the first time the server starts, and the UI says so.

**Already have SillyTavern?** The welcome screen scans the usual places (home, Desktop, Documents, Downloads, drive roots, the official launcher's `SillyTavern-Launcher\SillyTavern`) and offers to run an existing install in place ("adopted" mode), or you can browse to its folder. Nothing is copied or moved: the launcher runs `server.js` where it is with its own portable Node, runs the same `npm install --no-save` that `Start.bat` runs, adds only the phone whitelist to `config.yaml` (their `listen`, port and browser settings are left alone; ours are passed as command-line flags), and reads their port. `Start.bat`, their shortcuts and their git checkout keep working. If their old instance is already running, the launcher shows it as running instead of starting a second one.

Every run after that (the launcher):

- Starts `node server.js` with no console window, waits for the port to answer, opens the browser.
- Shows a QR code and the `http://192.168.x.x:8000` address for the phone, with "same Wi-Fi" and "add to home screen" instructions.
- Lives in the tray. Closing the window keeps SillyTavern running; Quit stops it. The Node process is attached to a Windows Job Object, so if the launcher dies for any reason Node dies with it and port 8000 is never left hostage.
- Checks for updates a few seconds after each start (toggleable) and shows a banner. SillyTavern updates follow the install: a git clone gets `git fetch` + commits-behind on its own branch and `git pull --ff-only` + `npm install` (staging users stay on staging); anything else gets the newest release installed next to the old one, then a folder swap that carries `data/`, `config.yaml`, `plugins/` (and `.git`) across, with `src/server-init.js` run afterwards so new config keys get added.
- Makes phone access debuggable without a terminal. The phone card picks the Wi-Fi/Ethernet address over VPN and virtual adapters (and labels the alternatives), reads SillyTavern's own log to say "Your iPhone connected at 15:42" or "a device at 192.168.1.50 was blocked [Allow it]", warns when the running copy was started outside the launcher (with a "Restart under the launcher" that closes the stray `node.exe` on the port), and has a "Check what's wrong" button: is the server bound to all interfaces, is the firewall allow rule there, is there a *block* rule for node.exe (the "clicked Cancel on the Windows popup" case), does a VPN own the default route, does `config.yaml` allow the home ranges, will the phone see a login (basic auth / user accounts). Every failing check carries a one-click fix; the firewall fix removes block rules and re-adds ours in a single UAC prompt. The rule itself now covers all private ranges (`10/8`, `172.16/12`, `192.168/16`) rather than just the local subnet.
- Keeps the PC awake while SillyTavern runs (toggle, on by default; the display may still sleep) and can start with Windows minimized to the tray (toggle, off by default; a Startup-folder shortcut with `--minimized`).
- Treats a half-finished install as not installed (npm writes `node_modules/.package-lock.json` last), keeps the install step list on the Rust side so a reopened window shows real progress, and offers "Repair installation" (re-runs the idempotent setup) wherever things look broken.
- Updates itself: the newest `SillyTavern-Launcher.exe` is downloaded from this repo's latest release, verified against the SHA-256 digest GitHub publishes for release assets, the running exe is renamed aside (Windows allows renaming a running exe, not overwriting it), the new one is moved into place and started with `--wait-for-pid <old pid>` so the single-instance guard does not bounce it, and the old one exits. The parked `.old.exe` is deleted on the next start.

Everything lives under `%LOCALAPPDATA%\SillyTavern\`:

```
%LOCALAPPDATA%\SillyTavern\
  SillyTavern Launcher.exe   copy of the launcher (shortcuts point here)
  node\                      portable Node.js
  SillyTavern\               the app; user content is in SillyTavern\data\
  npm-cache\
  logs\                      launcher-<time>.log per run, cli-<time>.log, crash-<time>-*.txt
  launcher-state.json        versions + launcher settings
```

## When a friend says "it doesn't work"

Ask them for the log zip. **Save logs to Desktop** sits in Maintenance, on the install/update error screen, next to "Repair installation" when the server fails, and in the tray icon's right-click menu (that one works even when the window is the broken part). It writes `SillyTavern-Launcher-logs-<time>.zip` to their Desktop and opens Explorer with it selected so they can drag it into Discord. If the launcher will not open at all: Win+R, `%LOCALAPPDATA%\SillyTavern\logs`, send the newest files.

What is in the zip:

- `summary.txt`: launcher, Windows and WebView2 versions, install mode and folder, SillyTavern and Node versions, launcher settings, server state, the `config.yaml` flags that matter (listen, whitelist mode and entries, basic auth on/off, user accounts, port), every network address with its adapter, what is listening on the port, and the full phone-access diagnosis.
- `logs/`: the newest 6 session logs (at most 30 MB before compression). Each session log starts with the Windows/WebView2 versions, the exe path and the settings. Uncaught errors from the webview land there too, as `UI error:` lines.
- `crashes/`: every crash report on disk.

What is never in it: `config.yaml` itself (basic-auth credentials), `secrets.json`, anything from SillyTavern's `data` folder, and chats. The user's profile path is rewritten to `%USERPROFILE%` and other `Users\<name>\` segments to `Users\<user>\`. SillyTavern's account-reset codes are blanked.

How the logging works:

- **One file per run.** The GUI writes `launcher-YYYYMMDD-HHMMSS.log`, `--cli` runs write `cli-...log`, and a file past 10 MB rolls over to `.2.log`, `.3.log`. Lines are buffered until the GUI knows it is the single instance, so double-clicking the shortcut while it runs leaves no stub file.
- **Retention** runs at every start: the newest 20 session logs stay, but nothing older than 30 days beyond the newest 5, and at most 64 MB in all. Crash reports: newest 10, nothing older than 90 days beyond the newest 3. Anything else in the folder is left alone. The single `launcher.log` older versions wrote is deleted on the first start of 0.4.0, because it could contain whole chats (see below).
- **Crash reports.** A panic hook writes `crash-<time>-panic.txt` with the message, the source location (`src\file.rs:line:col`), the thread and the last 200 log lines; release builds ship without a PDB, so the location is what pins it down. On top of that the GUI keeps a `logs\session.json` marker that is removed on Quit, on self-update and on `WM_ENDSESSION` (sign-out and shutdown, Fast Startup included). If the marker is still there at the next start, the previous run died without any of those (native crash, Task Manager, killed), and `crash-<time>-unclean-exit.txt` gets the tail of that run's log. If Windows booted after that run started, it was a restart or power loss and only gets a line in the new log.
- **SillyTavern runs at INFO.** Out of the box SillyTavern logs at DEBUG, and at DEBUG it prints the entire request body of every generation (system prompt, character card, chat history) to its console, which the launcher captures. The launcher starts it with `SILLYTAVERN_LOGGING_MINLOGLEVEL=1`; environment variables override `config.yaml`, so this holds for adopted installs too. The two lines the phone card parses survive: "New connection from" is INFO and "Blocked connection from" is WARN. Someone who wants SillyTavern's debug output can still run `Start.bat`.

## Building

Prerequisites on the build machine (all already present on this PC):

- Rust with the MSVC toolchain (`rust-toolchain.toml` pins `stable-x86_64-pc-windows-msvc`) and Visual Studio Build Tools with the Windows SDK.
- Node.js 20+ for the frontend toolchain (Vite + Svelte 5 + TypeScript). This is only a build-time dependency; the launcher downloads its own portable Node for SillyTavern.
- WebView2 runtime (ships with Windows 11; the launcher needs it at runtime too).
- Python + Pillow, only if you change the icon (`python tools/make_icons.py`).

```powershell
# from the repo root
npm install                # once
.\tools\build.ps1          # vite build + cargo build --release, copies the exe to release\

# development with hot reload (Vite dev server + debug Tauri window)
npm run tauri dev

# checks
npm run check              # svelte-check (TypeScript + Svelte)
cd src-tauri; cargo test   # config.yaml patcher tests
```

Tauri picks dev or production mode by the `custom-protocol` cargo feature, not by the cargo profile. Without it the app loads the Vite dev URL from `tauri.conf.json` (that is what `npm run tauri dev` relies on); with it the app serves the embedded `dist/`. `build.ps1` passes `--features custom-protocol` and runs `npm run build` first for exactly that reason. A bare `cargo build --release` produces an exe that tries to reach `localhost:1420` and shows "can't reach this page".

The release exe is the whole product. No MSI or NSIS bundle is produced (`bundle.active` is false in `tauri.conf.json`).

## Testing without touching your real install

`STLAUNCHER_HOME` overrides the install root. Combined with the CLI mode you can exercise the whole flow in a scratch folder:

```powershell
$env:STLAUNCHER_HOME = "D:\scratch\st-home"
.\release\SillyTavern Launcher.exe --cli install --no-shortcut --no-firewall
.\release\SillyTavern Launcher.exe --cli install --adopt D:\SillyTavern --no-shortcut --no-firewall   # run an existing install in place
.\release\SillyTavern Launcher.exe --cli discover     # list SillyTavern folders found on this PC
.\release\SillyTavern Launcher.exe --cli diagnose     # the phone-access checks, as text (good for "paste me the output")
.\release\SillyTavern Launcher.exe --cli status
.\release\SillyTavern Launcher.exe --cli start        # foreground, Ctrl+C to stop
.\release\SillyTavern Launcher.exe --cli check-update # SillyTavern (git or release) and the launcher itself
.\release\SillyTavern Launcher.exe --cli update
.\release\SillyTavern Launcher.exe --cli self-update [--force]
.\release\SillyTavern Launcher.exe --cli logs [--out D:\scratch]   # the support zip (default: Desktop)
.\release\SillyTavern Launcher.exe --cli crash-test   # deliberate panic: proves crash reports get written
```

Launching the exe with the same variable set opens the GUI against that folder. Debug builds keep a console window; release builds are GUI-subsystem and reattach to the parent console for `--cli`.

## Layout

```
index.html, vite.config.ts, svelte.config.js, tsconfig.json, package.json
src/                      Svelte 5 frontend (runes, TypeScript)
  main.ts                 mounts App
  app.css                 tokens + shared primitives (buttons, cards, type)
  App.svelte              screen switch: welcome / task / home
  lib/api.ts              typed invoke/listen wrappers; interfaces mirror the Rust structs
  lib/store.svelte.ts     the one reactive store (status, task steps, logs, toast)
  lib/format.ts           humanBytes
  lib/components/         TopBar, Welcome, TaskProgress, Home, UpdateBanner, StatusCard,
                          MaintenanceCard, PhoneCard, LogPanel, Toast
public/icon.png
src-tauri/src/
  lib.rs          entry: --cli dispatch or GUI
  gui.rs          Tauri builder, tray icon, single-instance, auto-start
  commands.rs     #[tauri::command]s + background server start helper
  installer.rs    install (managed or adopted) + update orchestration (idempotent steps)
  discover.rs     finds existing SillyTavern folders to adopt
  git.rs          branch / behind-count / ff-pull for adopted clones
  selfupdate.rs   launcher self-update (download, digest check, rename swap)
  diagnose.rs     phone-access checks with fix ids
  awake.rs        SetThreadExecutionState keep-awake worker
  node.rs         nodejs.org index lookup, download, checksum, unpack
  sillytavern.rs  GitHub latest-release lookup, archive download
  npm.rs          npm ci via the bundled node
  config.rs       line-based config.yaml patcher (keeps user edits + comments)
  server.rs       spawn node server.js, readiness watcher, connection feedback, take-over, stop
  process.rs      CREATE_NO_WINDOW helper + Job Object
  shortcut.rs     .lnk creation through IShellLink
  firewall.rs     netsh rule via one UAC prompt (ShellExecuteEx "runas"), block-rule detection and repair
  net.rs          adapter-aware LAN addresses, port probes, netstat/tasklist helpers, QR SVG
  download.rs     streaming download with progress + retries, sha256
  archive.rs      zip extraction that strips the top-level folder
  reporter.rs     step/progress events (Tauri + CLI implementations)
  logging.rs      per-session log files + retention, ring buffer, live event logger
  crash.rs        panic hook and the session marker that catches unclean exits
  support.rs      "Save logs to Desktop" zip: summary, recent logs, crash reports, redaction
  sysinfo.rs      Windows / WebView2 versions, boot time
  state.rs        launcher-state.json
  paths.rs        all paths derived from the install root
  cli.rs          --cli subcommands
tools/            make_icons.py, build.ps1
```

`docs/` is the download page served by GitHub Pages (Settings -> Pages -> `main` / `docs`). Its button links to `https://github.com/TrevorBrown-dev/sillytavern-launcher/releases/latest/download/SillyTavern-Launcher.exe`, a GitHub URL that always redirects to the newest release's asset of that name, so the page never needs updating when a release goes out; the script on the page only decorates the button with the version, size and date from the releases API. Keep the asset name `SillyTavern-Launcher.exe` stable across releases for that to hold.

The Rust <-> UI contract is small: commands in `commands.rs` (invoked through `api.ts`) and five events, `task-event` (install/update/self-update steps and progress), `server-event` (server state changes), `connection-event` (a phone connected or was blocked, parsed from SillyTavern's log), `updates-available` (the on-launch check) and `log-line`. If you add a field to a serde struct, add it to the matching interface in `api.ts`.

Releasing a new launcher version: bump `version` in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`, run `tools\build.ps1`, then `gh release create vX.Y.Z "release\SillyTavern-Launcher.exe" --title "SillyTavern Launcher X.Y.Z" --notes-file ...`. The asset must keep the name `SillyTavern-Launcher.exe`: the download page and the self-updater both rely on it.

## Decisions worth knowing

- **Self-contained over system installs.** The official launcher installs Git and Node through winget and drives a console menu. That is exactly what loses this audience, and it breaks on PATH problems and admin prompts. Portable Node in a private folder avoids all of it.
- **LAN access on by default, whitelist by private ranges.** Basic auth would be more secure but means typing a password on the phone; for a home network and a trusting audience, the IP whitelist plus a `LocalSubnet`-scoped firewall rule is the lower-friction choice. The toggle in the UI flips `listen` and restarts the server.
- **CLI flags plus config.yaml.** The server is started with `--listen=<bool> --browserLaunchEnabled=false --port N`, so the launcher's settings always win even if the user edits `config.yaml`, while the whitelist (which has no CLI flag) lives in the file.
- **Node 24 pinned by major.** `node::PREFERRED_MAJOR` picks the newest patch of that line; bump it when SillyTavern's `engines.node` moves. Falls back to the newest LTS if that line disappears from the index.
- **Updates never delete first.** The new version is fully installed into `SillyTavern.new` before anything is renamed, and `SillyTavern.old` is only removed after the swap succeeded.
- **Passive logs before feedback forms.** Friends report problems to Trevor directly, so the launcher only has to make the evidence easy to hand over: per-run logs, crash reports, a one-click zip. No telemetry, nothing is sent anywhere by the app. An in-app report form or an upload endpoint is the next step only if direct reports get out of hand.
- **Adopt, don't import.** Long-time users often have gigabytes of chats and characters; copying them is slow and leaves two diverging installs. Running their folder in place keeps one source of truth, and the launcher's footprint there is limited to things `Start.bat` already does plus whitelist entries. Updates respect their branch instead of forcing the release zip onto a git checkout.

## Known limitations

- Unsigned exe: SmartScreen shows "Windows protected your PC" and the user has to click "More info" then "Run anyway". Fine for friends; code signing (Azure Artifact Signing) would be the fix for wider distribution.
- Windows only. Mac would need Apple notarization; Android users have no path here at all.
- The phone address changes if the PC moves to a different Wi-Fi; the UI refreshes it every 15 s, and the `hostname.local` fallback helps on iPhones.
- If the PC sleeps, the phone loses access. Nothing the launcher can do about that beyond saying so.
- SillyTavern is AGPL-3.0. The launcher downloads it from upstream at install time and does not redistribute it.
