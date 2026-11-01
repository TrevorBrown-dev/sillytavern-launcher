/* Single reactive store for the launcher UI (Svelte 5 runes in a class).
 * Screens derive from `status` + `task`; events from Rust mutate the state here. */

import {
  api,
  errorText,
  events,
  type InstallOptions,
  type ServerStatus,
  type StatusInfo,
  type StepStatus,
  type TaskEvent,
  type TaskKind,
} from "./api";

export interface StepView {
  id: string;
  label: string;
  status: StepStatus;
  detail: string;
}

export interface ProgressView {
  done: number;
  total: number | null;
  unit: string;
}

export type Screen = "loading" | "welcome" | "task" | "home";

/** Step ids and labels, in the order the Rust installer reports them. */
export const STEP_SETS: Record<TaskKind, [id: string, label: string][]> = {
  install: [
    ["node", "Node.js runtime"],
    ["sillytavern", "SillyTavern"],
    ["deps", "Packages"],
    ["config", "Configuration"],
    ["shortcut", "Shortcuts"],
    ["firewall", "Firewall"],
  ],
  update: [
    ["sillytavern", "New version"],
    ["deps", "Packages"],
    ["config", "Switch over"],
  ],
};

const LOG_CAP = 600;
const STATUS_POLL_MS = 15_000;

class AppStore {
  status = $state<StatusInfo | null>(null);
  task = $state<TaskKind | null>(null);
  steps = $state<StepView[]>([]);
  progress = $state<ProgressView | null>(null);
  taskError = $state<string | null>(null);
  logLines = $state<string[]>([]);
  toast = $state<string | null>(null);
  lastInstallOptions = $state<InstallOptions>({
    lanEnabled: true,
    createShortcut: true,
    addFirewallRule: true,
  });

  screen: Screen = $derived.by(() => {
    if (this.task) return "task";
    if (!this.status) return "loading";
    return this.status.installed ? "home" : "welcome";
  });

  #lastServerEvent: ServerStatus | null = null;
  #lastServerEventAt = 0;
  #toastTimer: ReturnType<typeof setTimeout> | undefined;
  #logsLoaded = false;

  async init() {
    await events.onTask((e) => this.handleTaskEvent(e));
    await events.onServer((server) => {
      this.#lastServerEvent = server;
      this.#lastServerEventAt = Date.now();
      if (this.status) this.status.server = server;
      api.uiLog(`server-event: ${server.state}`);
    });
    await events.onLog((line) => this.appendLog(line));
    api.uiLog("listeners registered");
    await this.refreshStatus();
    // Keeps the phone address fresh (Wi-Fi changes) and the status honest.
    setInterval(() => {
      if (!this.task) void this.refreshStatus();
    }, STATUS_POLL_MS);
  }

  async refreshStatus() {
    const startedAt = Date.now();
    try {
      const status = await api.getStatus();
      // A server event that landed while get_status was in flight is newer than its answer.
      if (this.#lastServerEvent && this.#lastServerEventAt >= startedAt) {
        status.server = this.#lastServerEvent;
      }
      api.uiLog(`status: installed=${status.installed} busy=${status.busy} server=${status.server.state}`);
      this.status = status;
      // Window reopened mid-task: the step list is gone, so show a neutral task screen.
      if (status.busy && !this.task) this.beginTask("install");
    } catch (e) {
      this.showToast(`Could not read status: ${errorText(e)}`);
    }
  }

  showToast(message: string, ms = 2600) {
    this.toast = message;
    clearTimeout(this.#toastTimer);
    this.#toastTimer = setTimeout(() => {
      this.toast = null;
    }, ms);
  }

  // ---------- install / update ----------

  beginTask(kind: TaskKind) {
    this.task = kind;
    this.taskError = null;
    this.progress = null;
    this.steps = STEP_SETS[kind].map(([id, label]) => ({ id, label, status: "pending", detail: "" }));
  }

  async startInstall(options: InstallOptions) {
    this.lastInstallOptions = options;
    this.beginTask("install");
    try {
      await api.startInstall(options);
    } catch (e) {
      this.failTask(errorText(e));
    }
  }

  async startUpdate() {
    this.beginTask("update");
    try {
      await api.startUpdate();
    } catch (e) {
      this.failTask(errorText(e));
    }
  }

  retryTask() {
    if (this.task === "update") void this.startUpdate();
    else void this.startInstall(this.lastInstallOptions);
  }

  leaveTask() {
    this.task = null;
    void this.refreshStatus();
  }

  handleTaskEvent(e: TaskEvent) {
    if (e.kind === "step") {
      const step = this.steps.find((s) => s.id === e.step);
      if (step) {
        step.status = e.status;
        step.detail = e.detail;
      }
      if (e.status !== "running") this.progress = null;
    } else if (e.kind === "progress") {
      this.progress = { done: e.done, total: e.total, unit: e.unit };
    } else if (e.ok) {
      this.showToast(
        this.task === "install"
          ? "SillyTavern is installed. Starting it now..."
          : "Update complete. Starting SillyTavern...",
      );
      this.task = null;
      void this.refreshStatus();
    } else {
      this.failTask(e.error ?? "Unknown error");
    }
  }

  failTask(message: string) {
    for (const step of this.steps) {
      if (step.status === "running") step.status = "error";
    }
    this.progress = null;
    this.taskError = message;
  }

  // ---------- logs ----------

  appendLog(line: string) {
    this.logLines.push(line);
    if (this.logLines.length > LOG_CAP) {
      this.logLines.splice(0, this.logLines.length - LOG_CAP);
    }
  }

  /** Pulls the recent history from Rust the first time a log panel opens. */
  async loadLogs() {
    if (this.#logsLoaded) return;
    try {
      this.logLines = await api.getLogs(400);
      this.#logsLoaded = true;
    } catch {
      // the live stream still works without history
    }
  }
}

export const app = new AppStore();
