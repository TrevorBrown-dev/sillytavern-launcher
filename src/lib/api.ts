/* Typed bridge to the Rust side. The interfaces mirror the serde structs in
 * src-tauri/src/{commands,server,state,installer,discover,selfupdate,reporter,diagnose}.rs
 * (camelCase on the wire). */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ServerState = "stopped" | "starting" | "running" | "error";

export interface ServerStatus {
  state: ServerState;
  url: string;
  message: string | null;
  pid: number | null;
  /** Something we did not start is already serving SillyTavern on the port. */
  external: boolean;
}

export interface Settings {
  lanEnabled: boolean;
  port: number;
  openBrowserOnStart: boolean;
  checkUpdatesOnStart: boolean;
  keepAwake: boolean;
}

export type InstallMode = "managed" | "adopted";

export interface AltAddress {
  url: string;
  adapter: string;
  virtualAdapter: boolean;
}

export interface PhoneInfo {
  enabled: boolean;
  url: string | null;
  adapter: string | null;
  qrSvg: string | null;
  alternatives: AltAddress[];
  hostnameUrl: string | null;
  /** A VPN/virtual adapter owns the default route; the shown address is the Wi-Fi one. */
  vpnActive: boolean;
}

export interface ConnectionEvent {
  ip: string;
  userAgent: string;
  blocked: boolean;
  /** "15:42" */
  at: string;
  /** "iPhone", "iPad", "Android phone", "a phone", "another device" */
  device: string;
}

export interface Connections {
  lastRemote: ConnectionEvent | null;
  lastBlocked: ConnectionEvent | null;
}

export type TaskKind = "install" | "update" | "selfupdate";
export type StepStatus = "pending" | "running" | "done" | "skipped" | "error";

export interface TaskSnapshot {
  task: TaskKind | null;
  steps: { step: string; status: StepStatus; detail: string }[];
  progress: { step: string; done: number; total: number | null; unit: string } | null;
  finished: { ok: boolean; error: string | null } | null;
}

export interface StatusInfo {
  installed: boolean;
  busy: boolean;
  mode: InstallMode;
  stDir: string;
  isGit: boolean;
  branch: string | null;
  stVersion: string | null;
  nodeVersion: string | null;
  settings: Settings;
  server: ServerStatus;
  phone: PhoneInfo;
  connections: Connections;
  task: TaskSnapshot;
  startWithWindows: boolean;
  home: string;
  launcherVersion: string;
  firewallRule: boolean;
}

/** A SillyTavern folder the user already has (found by scanning or picked by hand). */
export interface ExistingInstall {
  path: string;
  version: string | null;
  isGit: boolean;
  branch: string | null;
  port: number;
  hasNodeModules: boolean;
  hasData: boolean;
}

export type TaskEvent =
  | { kind: "step"; task: TaskKind; step: string; status: StepStatus; detail: string }
  | { kind: "progress"; task: TaskKind; step: string; done: number; total: number | null; unit: string }
  | { kind: "finished"; task: TaskKind; ok: boolean; error: string | null };

export interface InstallOptions {
  lanEnabled: boolean;
  createShortcut: boolean;
  addFirewallRule: boolean;
  /** Run this existing folder in place instead of installing a fresh copy. */
  adoptPath?: string | null;
}

export interface UpdateCheck {
  installed: string | null;
  latest: string;
  available: boolean;
  method: "git" | "release";
  branch: string | null;
  detail: string | null;
}

export interface LauncherUpdate {
  current: string;
  latest: string;
  available: boolean;
  url: string | null;
  size: number | null;
  sha256: string | null;
  notesUrl: string | null;
}

export interface UpdateReport {
  sillytavern: UpdateCheck | null;
  launcher: LauncherUpdate | null;
  errors: string[];
}

export type CheckLevel = "ok" | "info" | "warn" | "fail";
export type FixId = "lan" | "start" | "takeover" | "firewall" | "whitelist";

export interface Check {
  id: string;
  level: CheckLevel;
  title: string;
  detail: string;
  fix: FixId | null;
  fixLabel: string | null;
}

export interface Diagnosis {
  checks: Check[];
  ok: boolean;
  summary: string;
}

export const api = {
  getStatus: () => invoke<StatusInfo>("get_status"),
  discoverInstalls: () => invoke<ExistingInstall[]>("discover_installs"),
  /** Opens the native folder picker; resolves null when cancelled, rejects when not SillyTavern. */
  pickInstallFolder: () => invoke<ExistingInstall | null>("pick_install_folder"),
  startInstall: (options: InstallOptions) => invoke<void>("start_install", { options }),
  repairInstall: () => invoke<void>("repair_install"),
  startServer: () => invoke<void>("start_server"),
  stopServer: () => invoke<void>("stop_server"),
  takeOverServer: () => invoke<void>("take_over_server"),
  openBrowser: () => invoke<void>("open_browser"),
  setLanEnabled: (enabled: boolean) => invoke<StatusInfo>("set_lan_enabled", { enabled }),
  allowIp: (ip: string) => invoke<StatusInfo>("allow_ip", { ip }),
  diagnosePhone: () => invoke<Diagnosis>("diagnose_phone"),
  fixFirewall: () => invoke<string>("fix_firewall"),
  fixWhitelist: () => invoke<StatusInfo>("fix_whitelist"),
  setOpenBrowser: (enabled: boolean) => invoke<void>("set_open_browser", { enabled }),
  setCheckUpdatesOnStart: (enabled: boolean) => invoke<void>("set_check_updates_on_start", { enabled }),
  setKeepAwake: (enabled: boolean) => invoke<void>("set_keep_awake", { enabled }),
  setStartWithWindows: (enabled: boolean) => invoke<void>("set_start_with_windows", { enabled }),
  checkUpdates: () => invoke<UpdateReport>("check_updates"),
  startUpdate: () => invoke<void>("start_update"),
  startSelfUpdate: () => invoke<void>("start_self_update"),
  openInstallFolder: () => invoke<void>("open_install_folder"),
  openConfigFile: () => invoke<void>("open_config_file"),
  getLogs: (limit = 400) => invoke<string[]>("get_logs", { limit }),
  createShortcuts: () => invoke<string>("create_shortcuts"),
  addFirewallRule: () => invoke<string>("add_firewall_rule"),
  quit: () => invoke<void>("quit_app"),
  /** Zips recent logs + crash reports onto the Desktop and shows it in Explorer; resolves with a message. */
  saveSupportBundle: () => invoke<string>("save_support_bundle"),
  openLogsFolder: () => invoke<void>("open_logs_folder"),
  /** Breadcrumbs into the session log; never throws. */
  uiLog: (line: string) => invoke<void>("ui_log", { line }).catch(() => undefined),
  /** Uncaught errors from the webview, logged at error level; never throws. */
  uiError: (line: string) => invoke<void>("ui_log", { line, level: "error" }).catch(() => undefined),
};

export const events = {
  onTask: (cb: (event: TaskEvent) => void): Promise<UnlistenFn> =>
    listen<TaskEvent>("task-event", (e) => cb(e.payload)),
  onServer: (cb: (status: ServerStatus) => void): Promise<UnlistenFn> =>
    listen<ServerStatus>("server-event", (e) => cb(e.payload)),
  onConnection: (cb: (event: ConnectionEvent) => void): Promise<UnlistenFn> =>
    listen<ConnectionEvent>("connection-event", (e) => cb(e.payload)),
  onLog: (cb: (line: string) => void): Promise<UnlistenFn> =>
    listen<string>("log-line", (e) => cb(e.payload)),
  onUpdates: (cb: (report: UpdateReport) => void): Promise<UnlistenFn> =>
    listen<UpdateReport>("updates-available", (e) => cb(e.payload)),
};

/** Tauri rejects with the Rust `Err(String)` itself, so unwrap both shapes. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
