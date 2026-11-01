# SillyTavern Launcher

A single Windows `.exe` that installs SillyTavern end to end for someone who has never opened a terminal, then stays around as the thing they double-click to run it. Built for friends coming over from JanitorAI who mostly want to use it from their phone.

## What it does

First run (the installer):

1. Downloads a portable Node.js (the newest 24.x) from nodejs.org, verifies the SHA-256 against `SHASUMS256.txt`, unpacks it. Nothing is added to the system PATH and no admin rights are needed.
2. Downloads the latest SillyTavern release zip from GitHub (no Git involved) and unpacks it.
3. Runs `npm ci --omit=dev --ignore-scripts` with the bundled Node, the same flags SillyTavern's own `Start.bat` uses.
4. Writes `config.yaml` from SillyTavern's defaults with `listen: true`, whitelist mode on, and the private IPv4 ranges (`10/8`, `172.16/12`, `192.168/16`) whitelisted, so phones on the home Wi-Fi can connect without typing IPs anywhere. SillyTavern's own browser auto-launch is turned off because the launcher handles that.
5. Copies itself into the install folder and creates Desktop + Start Menu shortcuts pointing at that copy (so deleting the original download does not break anything).
6. Adds an inbound Windows Firewall rule for the bundled `node.exe`, limited to `LocalSubnet`. This is the one step that needs admin, so it triggers a single UAC prompt. If the user declines, Windows will show its own "allow Node.js" prompt the first time the server starts, and the UI says so.

Every run after that (the launcher):

- Starts `node server.js` with no console window, waits for the port to answer, opens the browser.
- Shows a QR code and the `http://192.168.x.x:8000` address for the phone, with "same Wi-Fi" and "add to home screen" instructions.
- Lives in the tray. Closing the window keeps SillyTavern running; Quit stops it. The Node process is attached to a Windows Job Object, so if the launcher dies for any reason Node dies with it and port 8000 is never left hostage.
- Can update SillyTavern in place: the new release is downloaded and installed next to the old one, then folders are swapped and `data/`, `config.yaml` and `plugins/` are carried across. `src/server-init.js` is run afterwards so new config keys get added.

Everything lives under `%LOCALAPPDATA%\SillyTavern\`:

```
%LOCALAPPDATA%\SillyTavern\
  SillyTavern Launcher.exe   copy of the launcher (shortcuts point here)
  node\                      portable Node.js
  SillyTavern\               the app; user content is in SillyTavern\data\
  npm-cache\
  logs\launcher.log
  launcher-state.json        versions + launcher settings
```

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
.\release\SillyTavern Launcher.exe --cli status
.\release\SillyTavern Launcher.exe --cli start        # foreground, Ctrl+C to stop
.\release\SillyTavern Launcher.exe --cli check-update
.\release\SillyTavern Launcher.exe --cli update
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
  lib/components/         TopBar, Welcome, TaskProgress, Home, StatusCard,
                          MaintenanceCard, PhoneCard, LogPanel, Toast
public/icon.png
src-tauri/src/
  lib.rs          entry: --cli dispatch or GUI
  gui.rs          Tauri builder, tray icon, single-instance, auto-start
  commands.rs     #[tauri::command]s + background server start helper
  installer.rs    install + update orchestration (idempotent steps)
  node.rs         nodejs.org index lookup, download, checksum, unpack
  sillytavern.rs  GitHub latest-release lookup, archive download
  npm.rs          npm ci via the bundled node
  config.rs       line-based config.yaml patcher (keeps user edits + comments)
  server.rs       spawn node server.js, readiness watcher, stop
  process.rs      CREATE_NO_WINDOW helper + Job Object
  shortcut.rs     .lnk creation through IShellLink
  firewall.rs     netsh rule via one UAC prompt (ShellExecuteEx "runas")
  net.rs          LAN IP discovery, port probe, QR SVG
  download.rs     streaming download with progress + retries, sha256
  archive.rs      zip extraction that strips the top-level folder
  reporter.rs     step/progress events (Tauri + CLI implementations)
  logging.rs      file + ring buffer + live event logger
  state.rs        launcher-state.json
  paths.rs        all paths derived from the install root
  cli.rs          --cli subcommands
tools/            make_icons.py, build.ps1
```

The Rust <-> UI contract is small: commands in `commands.rs` (invoked through `api.ts`) and three events, `task-event` (install/update steps and progress), `server-event` (server state changes) and `log-line`. If you add a field to a serde struct, add it to the matching interface in `api.ts`.

## Decisions worth knowing

- **Self-contained over system installs.** The official launcher installs Git and Node through winget and drives a console menu. That is exactly what loses this audience, and it breaks on PATH problems and admin prompts. Portable Node in a private folder avoids all of it.
- **LAN access on by default, whitelist by private ranges.** Basic auth would be more secure but means typing a password on the phone; for a home network and a trusting audience, the IP whitelist plus a `LocalSubnet`-scoped firewall rule is the lower-friction choice. The toggle in the UI flips `listen` and restarts the server.
- **CLI flags plus config.yaml.** The server is started with `--listen=<bool> --browserLaunchEnabled=false --port N`, so the launcher's settings always win even if the user edits `config.yaml`, while the whitelist (which has no CLI flag) lives in the file.
- **Node 24 pinned by major.** `node::PREFERRED_MAJOR` picks the newest patch of that line; bump it when SillyTavern's `engines.node` moves. Falls back to the newest LTS if that line disappears from the index.
- **Updates never delete first.** The new version is fully installed into `SillyTavern.new` before anything is renamed, and `SillyTavern.old` is only removed after the swap succeeded.

## Known limitations

- Unsigned exe: SmartScreen shows "Windows protected your PC" and the user has to click "More info" then "Run anyway". Fine for friends; code signing (Azure Artifact Signing) would be the fix for wider distribution.
- Windows only. Mac would need Apple notarization; Android users have no path here at all.
- The phone address changes if the PC moves to a different Wi-Fi; the UI refreshes it every 15 s, and the `hostname.local` fallback helps on iPhones.
- If the PC sleeps, the phone loses access. Nothing the launcher can do about that beyond saying so.
- SillyTavern is AGPL-3.0. The launcher downloads it from upstream at install time and does not redistribute it.
