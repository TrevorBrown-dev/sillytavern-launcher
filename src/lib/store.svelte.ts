/* Single reactive store for the launcher UI (Svelte 5 runes in a class).
 * Screens derive from `status` + `task`; events from Rust mutate the state here. */

import {
  api,
  errorText,
  events,
  type Diagnosis,
  type FixId,
  type InstallOptions,
  type ServerStatus,
  type StatusInfo,
  type StepStatus,
  type TaskEvent,
  type TaskKind,
  type UpdateReport,
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

/** Step ids and labels, in the order the Rust side reports them. */
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
  selfupdate: [
    ["download", "Download the new launcher"],
    ["verify", "Verify"],
    ["swap", "Install"],
    ["restart", "Restart"],
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
  updates = $state<UpdateReport | null>(null);
  updatesDismissed = $state(false);
  checkingUpdates = $state(false);
  diagnosis = $state<Diagnosis | null>(null);
  diagnosing = $state(false);
  fixing = $state<FixId | null>(null);
  lastInstallOptions = $state<InstallOptions>({
    lanEnabled: true,
    createShortcut: true,
    addFirewallRule: true,
    adoptPath: null,
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
    await events.onConnection((ev) => {
      if (this.status) {
        if (ev.blocked) this.status.connections.lastBlocked = ev;
        else this.status.connections.lastRemote = ev;
      }
      api.uiLog(`connection-event: ${ev.blocked ? "blocked" : "ok"} ${ev.ip} (${ev.device})`);
      this.showToast(
        ev.blocked
          ? `${ev.device} at ${ev.ip} tried to connect and was blocked. See "On your phone".`
          : `Your ${ev.device} connected.`,
        ev.blocked ? 6000 : 3500,
      );
    });
    await events.onLog((line) => this.appendLog(line));
    await events.onUpdates((report) => {
      this.updates = report;
      this.updatesDismissed = false;
      api.uiLog(
        `updates-available: st=${report.sillytavern?.available ?? "?"} launcher=${report.launcher?.available ?? "?"}`,
      );
    });
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
      // Window reopened mid-task: rebuild the step list from the Rust-side snapshot.
      if (status.busy && !this.task) this.restoreTask(status);
    } catch (e) {
      this.showToast(`Could not read status: ${errorText(e)}`);
    }
  }

  restoreTask(status: StatusInfo) {
    const kind = status.task.task ?? "install";
    this.beginTask(kind);
    for (const snap of status.task.steps) {
      const step = this.steps.find((s) => s.id === snap.step);
      if (step) {
        step.status = snap.status;
        step.detail = snap.detail;
      }
    }
    if (status.task.progress) {
      const p = status.task.progress;
      this.progress = { done: p.done, total: p.total, unit: p.unit };
    }
  }

  showToast(message: string, ms = 2600) {
    this.toast = message;
    clearTimeout(this.#toastTimer);
    this.#toastTimer = setTimeout(() => {
      this.toast = null;
    }, ms);
  }

  // ---------- install / update / self-update ----------

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

  /** Re-runs setup for the current install; fixes half-finished installs in place. */
  async repairInstall() {
    const s = this.status;
    this.lastInstallOptions = {
      lanEnabled: s?.settings.lanEnabled ?? true,
      createShortcut: true,
      addFirewallRule: s?.settings.lanEnabled ?? true,
      adoptPath: s?.mode === "adopted" ? s.stDir : null,
    };
    this.beginTask("install");
    try {
      await api.repairInstall();
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

  async startSelfUpdate() {
    this.beginTask("selfupdate");
    try {
      await api.startSelfUpdate();
    } catch (e) {
      this.failTask(errorText(e));
    }
  }

  retryTask() {
    if (this.task === "update") void this.startUpdate();
    else if (this.task === "selfupdate") void this.startSelfUpdate();
    else void this.startInstall(this.lastInstallOptions);
  }

  leaveTask() {
    this.task = null;
    void this.refreshStatus();
  }

  async checkUpdates() {
    this.checkingUpdates = true;
    try {
      this.updates = await api.checkUpdates();
      this.updatesDismissed = false;
    } catch (e) {
      this.showToast(`Could not check for updates: ${errorText(e)}`);
    } finally {
      this.checkingUpdates = false;
    }
  }

  // ---------- phone access helpers ----------

  async runDiagnosis() {
    this.diagnosing = true;
    try {
      this.diagnosis = await api.diagnosePhone();
    } catch (e) {
      this.showToast(`Could not run the check: ${errorText(e)}`);
    } finally {
      this.diagnosing = false;
    }
  }

  /** Runs one of the fixes the diagnosis offers, then re-checks. */
  async applyFix(fix: FixId) {
    this.fixing = fix;
    try {
      switch (fix) {
        case "lan":
          this.status = await api.setLanEnabled(true);
          break;
        case "start":
          await api.startServer();
          await new Promise((r) => setTimeout(r, 4000));
          break;
        case "takeover":
          await api.takeOverServer();
          await new Promise((r) => setTimeout(r, 4000));
          break;
        case "firewall":
          this.showToast(await api.fixFirewall(), 5000);
          break;
        case "whitelist":
          this.status = await api.fixWhitelist();
          break;
      }
      await this.refreshStatus();
      await this.runDiagnosis();
    } catch (e) {
      this.showToast(errorText(e), 6000);
    } finally {
      this.fixing = null;
    }
  }

  async takeOver() {
    try {
      await api.takeOverServer();
      this.showToast("Restarting SillyTavern under the launcher...");
    } catch (e) {
      this.showToast(errorText(e), 6000);
    }
  }

  async allowIp(ip: string) {
    try {
      this.status = await api.allowIp(ip);
      this.showToast(`${ip} is now allowed. Try again on the phone.`, 5000);
    } catch (e) {
      this.showToast(errorText(e), 6000);
    }
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
      if (this.task === "selfupdate") {
        // The new launcher is starting; this window is about to close.
        this.showToast("New launcher starting...", 10_000);
        return;
      }
      this.showToast(
        this.task === "install"
          ? "SillyTavern is ready. Starting it now..."
          : "Update complete. Starting SillyTavern...",
      );
      this.task = null;
      this.updates = null;
      this.diagnosis = null;
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
