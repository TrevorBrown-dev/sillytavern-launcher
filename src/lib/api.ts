/* Typed bridge to the Rust side. The interfaces mirror the serde structs in
 * src-tauri/src/{commands,server,state,installer,reporter}.rs (camelCase on the wire). */

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
}

export interface PhoneInfo {
  enabled: boolean;
  url: string | null;
  qrSvg: string | null;
  alternatives: string[];
  hostnameUrl: string | null;
}

export interface StatusInfo {
  installed: boolean;
  busy: boolean;
  stVersion: string | null;
  nodeVersion: string | null;
  settings: Settings;
  server: ServerStatus;
  phone: PhoneInfo;
  home: string;
  launcherVersion: string;
  firewallRule: boolean;
}

export type TaskKind = "install" | "update";
export type StepStatus = "pending" | "running" | "done" | "skipped" | "error";

export type TaskEvent =
  | { kind: "step"; task: TaskKind; step: string; status: StepStatus; detail: string }
  | { kind: "progress"; task: TaskKind; step: string; done: number; total: number | null; unit: string }
  | { kind: "finished"; task: TaskKind; ok: boolean; error: string | null };

export interface InstallOptions {
  lanEnabled: boolean;
  createShortcut: boolean;
  addFirewallRule: boolean;
}

export interface UpdateCheck {
  installed: string | null;
  latest: string;
  available: boolean;
}

export const api = {
  getStatus: () => invoke<StatusInfo>("get_status"),
  startInstall: (options: InstallOptions) => invoke<void>("start_install", { options }),
  startServer: () => invoke<void>("start_server"),
  stopServer: () => invoke<void>("stop_server"),
  openBrowser: () => invoke<void>("open_browser"),
  setLanEnabled: (enabled: boolean) => invoke<StatusInfo>("set_lan_enabled", { enabled }),
  setOpenBrowser: (enabled: boolean) => invoke<void>("set_open_browser", { enabled }),
  checkUpdate: () => invoke<UpdateCheck>("check_update"),
  startUpdate: () => invoke<void>("start_update"),
  openInstallFolder: () => invoke<void>("open_install_folder"),
  getLogs: (limit = 400) => invoke<string[]>("get_logs", { limit }),
  createShortcuts: () => invoke<string>("create_shortcuts"),
  addFirewallRule: () => invoke<string>("add_firewall_rule"),
  quit: () => invoke<void>("quit_app"),
  /** Breadcrumbs into launcher.log; never throws. */
  uiLog: (line: string) => invoke<void>("ui_log", { line }).catch(() => undefined),
};

export const events = {
  onTask: (cb: (event: TaskEvent) => void): Promise<UnlistenFn> =>
    listen<TaskEvent>("task-event", (e) => cb(e.payload)),
  onServer: (cb: (status: ServerStatus) => void): Promise<UnlistenFn> =>
    listen<ServerStatus>("server-event", (e) => cb(e.payload)),
  onLog: (cb: (line: string) => void): Promise<UnlistenFn> =>
    listen<string>("log-line", (e) => cb(e.payload)),
};

/** Tauri rejects with the Rust `Err(String)` itself, so unwrap both shapes. */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
