/* SillyTavern Launcher UI. Vanilla JS; talks to Rust through window.__TAURI__. */
(() => {
  "use strict";

  const tauri = window.__TAURI__;
  const invoke = tauri ? tauri.core.invoke : async (cmd) => {
    console.warn(`invoke(${cmd}) outside Tauri`);
    return null;
  };
  const listen = tauri ? tauri.event.listen : async () => () => {};

  const $ = (id) => document.getElementById(id);

  const STEP_SETS = {
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

  const STEP_GLYPH = { pending: "", running: "", done: "✓", skipped: "–", error: "!" };

  const state = {
    status: null,
    task: null, // "install" | "update" | null
    taskFailed: false,
    lastServerEvent: null,
    lastServerEventAt: 0,
  };

  // ---------- helpers ----------

  function uiLog(message) {
    invoke("ui_log", { line: message }).catch(() => {});
  }

  let toastTimer = null;
  function toast(message, ms = 2600) {
    const el = $("toast");
    el.textContent = message;
    el.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      el.hidden = true;
    }, ms);
  }

  function showScreen(name) {
    for (const s of document.querySelectorAll(".screen")) {
      s.hidden = s.id !== `screen-${name}`;
    }
  }

  function humanBytes(n) {
    const mb = 1024 * 1024;
    if (n >= 10 * mb) return `${Math.round(n / mb)} MB`;
    if (n >= mb) return `${(n / mb).toFixed(1)} MB`;
    return `${Math.round(n / 1024)} KB`;
  }

  function appendLog(line) {
    for (const id of ["log-task", "log-home"]) {
      const box = $(id);
      const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
      box.textContent += `${line}\n`;
      if (box.textContent.length > 120000) {
        box.textContent = box.textContent.slice(-90000);
      }
      if (atBottom) box.scrollTop = box.scrollHeight;
    }
  }

  async function loadLogs() {
    const lines = (await invoke("get_logs", { limit: 400 })) || [];
    for (const id of ["log-task", "log-home"]) {
      const box = $(id);
      box.textContent = `${lines.join("\n")}\n`;
      box.scrollTop = box.scrollHeight;
    }
  }

  // ---------- task screen (install / update) ----------

  function setupTask(kind) {
    state.task = kind;
    state.taskFailed = false;
    $("task-title").textContent = kind === "install" ? "Installing SillyTavern" : "Updating SillyTavern";
    $("task-subtitle").textContent =
      kind === "install"
        ? "Grab a drink. Downloading and unpacking takes a few minutes, mostly on the Packages step."
        : "Your chats, characters and settings are kept. This takes a few minutes.";
    const ol = $("steps");
    ol.innerHTML = "";
    for (const [id, label] of STEP_SETS[kind]) {
      const li = document.createElement("li");
      li.className = "step";
      li.dataset.step = id;
      li.dataset.status = "pending";
      const icon = document.createElement("span");
      icon.className = "step-icon";
      const text = document.createElement("div");
      const strong = document.createElement("span");
      strong.className = "step-label";
      strong.textContent = label;
      const detail = document.createElement("span");
      detail.className = "step-detail";
      text.append(strong, detail);
      li.append(icon, text);
      ol.append(li);
    }
    $("progress").hidden = true;
    $("task-error").hidden = true;
    $("btn-task-home").hidden = kind === "install";
    showScreen("task");
  }

  function onStep({ step, status, detail }) {
    const li = document.querySelector(`.step[data-step="${step}"]`);
    if (!li) return;
    li.dataset.status = status;
    li.querySelector(".step-icon").textContent = STEP_GLYPH[status] || "";
    li.querySelector(".step-detail").textContent = detail || "";
    if (status !== "running") {
      $("progress").hidden = true;
    }
  }

  function onProgress({ done, total, unit }) {
    const wrap = $("progress");
    const bar = $("progress-bar");
    wrap.hidden = false;
    if (total && total > 0) {
      bar.classList.remove("indeterminate");
      bar.style.width = `${Math.min(100, (done / total) * 100).toFixed(1)}%`;
      $("progress-text").textContent =
        unit === "bytes"
          ? `${humanBytes(done)} of ${humanBytes(total)}`
          : `${done.toLocaleString()} of ${total.toLocaleString()} ${unit}`;
    } else {
      bar.classList.add("indeterminate");
      bar.style.width = "";
      $("progress-text").textContent = unit === "bytes" ? `${humanBytes(done)} downloaded` : `${done}`;
    }
  }

  function onFinished({ ok, error }) {
    if (ok) {
      toast(state.task === "install" ? "SillyTavern is installed. Starting it now..." : "Update complete. Starting SillyTavern...");
      state.task = null;
      refreshStatus().then(() => showScreen("home"));
      return;
    }
    state.taskFailed = true;
    for (const li of document.querySelectorAll('.step[data-status="running"]')) {
      li.dataset.status = "error";
      li.querySelector(".step-icon").textContent = STEP_GLYPH.error;
    }
    $("progress").hidden = true;
    $("task-error-text").textContent = error || "Unknown error";
    $("task-error").hidden = false;
  }

  async function startInstall() {
    const options = {
      lanEnabled: $("opt-lan").checked,
      createShortcut: $("opt-shortcut").checked,
      addFirewallRule: $("opt-lan").checked,
    };
    setupTask("install");
    try {
      await invoke("start_install", { options });
    } catch (e) {
      onFinished({ ok: false, error: String(e) });
    }
  }

  async function startUpdate() {
    setupTask("update");
    try {
      await invoke("start_update");
    } catch (e) {
      onFinished({ ok: false, error: String(e) });
    }
  }

  // ---------- home screen ----------

  function renderServer(server) {
    const dot = $("status-dot");
    dot.className = `dot ${server.state}`;
    const titles = {
      stopped: "Stopped",
      starting: "Starting up...",
      running: "Running",
      error: "Problem",
    };
    $("status-title").textContent = titles[server.state] || server.state;
    const sub = {
      stopped: "SillyTavern is not running.",
      starting: "Waiting for SillyTavern to answer. First start can take a minute.",
      running: server.external ? `Already running at ${server.url}` : `Open at ${server.url}`,
      error: "SillyTavern could not start.",
    };
    $("status-sub").textContent = sub[server.state] || "";
    const msg = $("status-message");
    if (server.state === "error" && server.message) {
      msg.textContent = server.message;
      msg.hidden = false;
    } else {
      msg.hidden = true;
    }
    $("btn-open").disabled = server.state !== "running";
    $("btn-start").disabled = server.state === "running" || server.state === "starting";
    $("btn-stop").disabled = server.state === "stopped" || (server.external && server.state === "running");
  }

  function renderPhone(phone, settings) {
    $("toggle-lan").checked = settings.lanEnabled;
    $("phone-on").hidden = !settings.lanEnabled;
    $("phone-off").hidden = settings.lanEnabled;
    if (!settings.lanEnabled) return;
    $("qr").innerHTML = phone.qrSvg || "";
    $("phone-url").textContent = phone.url || "No Wi-Fi address found";
    const alt = $("phone-alt");
    const extras = [...(phone.alternatives || [])];
    if (phone.hostnameUrl) extras.push(`${phone.hostnameUrl} (works on iPhone and most Android phones)`);
    if (extras.length) {
      alt.textContent = `If that one doesn't work, try: ${extras.join("  ·  ")}`;
      alt.hidden = false;
    } else {
      alt.hidden = true;
    }
  }

  function applyStatus(status) {
    state.status = status;
    $("version-pill").textContent = `Launcher ${status.launcherVersion}`;
    $("version-pill").hidden = false;
    $("install-path").textContent = status.home;
    $("st-version").textContent = status.stVersion || "?";
    $("node-version").textContent = status.nodeVersion || "?";
    $("toggle-browser").checked = status.settings.openBrowserOnStart;
    renderServer(status.server);
    renderPhone(status.phone, status.settings);

    if (state.task) return; // a task screen is showing; events drive it
    if (status.busy) {
      // Window was reopened mid-task; we lost the step list, so show a neutral task screen.
      setupTask("install");
      return;
    }
    showScreen(status.installed ? "home" : "welcome");
  }

  async function refreshStatus() {
    const startedAt = Date.now();
    try {
      const status = await invoke("get_status");
      if (!status) return;
      // A server event that landed while get_status was in flight is newer than its answer.
      if (state.lastServerEvent && state.lastServerEventAt >= startedAt) {
        status.server = state.lastServerEvent;
      }
      uiLog(`status: installed=${status.installed} busy=${status.busy} server=${status.server.state}`);
      applyStatus(status);
    } catch (e) {
      toast(`Could not read status: ${e}`);
    }
  }

  async function checkUpdate() {
    const box = $("update-box");
    const text = $("update-text");
    const btn = $("btn-do-update");
    box.hidden = false;
    btn.hidden = true;
    text.textContent = "Checking...";
    try {
      const info = await invoke("check_update");
      if (info.available) {
        text.textContent = `Version ${info.latest} is available (you have ${info.installed || "an unknown version"}).`;
        btn.hidden = false;
      } else {
        text.textContent = `You're on the latest version (${info.latest}).`;
      }
    } catch (e) {
      text.textContent = `Could not check: ${e}`;
    }
  }

  async function copyUrl() {
    const url = $("phone-url").textContent;
    try {
      await navigator.clipboard.writeText(url);
      toast("Copied");
    } catch {
      const range = document.createRange();
      range.selectNodeContents($("phone-url"));
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(range);
      toast("Select and copy the address");
    }
  }

  // ---------- wiring ----------

  function wire() {
    $("btn-install").addEventListener("click", startInstall);
    $("btn-retry").addEventListener("click", () => (state.task === "update" ? startUpdate() : startInstall()));
    $("btn-task-home").addEventListener("click", () => {
      state.task = null;
      refreshStatus();
    });
    $("btn-open").addEventListener("click", () => invoke("open_browser"));
    $("btn-start").addEventListener("click", async () => {
      try {
        await invoke("start_server");
      } catch (e) {
        toast(String(e));
      }
    });
    $("btn-stop").addEventListener("click", async () => {
      try {
        await invoke("stop_server");
      } catch (e) {
        toast(String(e));
      }
    });
    $("btn-quit").addEventListener("click", () => invoke("quit_app"));
    $("btn-copy").addEventListener("click", copyUrl);
    $("btn-folder").addEventListener("click", () => invoke("open_install_folder"));
    $("btn-update").addEventListener("click", checkUpdate);
    $("btn-do-update").addEventListener("click", startUpdate);
    $("btn-shortcut").addEventListener("click", async () => {
      try {
        toast(await invoke("create_shortcuts"));
      } catch (e) {
        toast(String(e));
      }
    });
    $("btn-firewall").addEventListener("click", async () => {
      try {
        toast(await invoke("add_firewall_rule"));
      } catch (e) {
        toast(String(e));
      }
    });
    $("toggle-lan").addEventListener("change", async (ev) => {
      const enabled = ev.target.checked;
      ev.target.disabled = true;
      try {
        const status = await invoke("set_lan_enabled", { enabled });
        if (status) applyStatus(status);
        toast(enabled ? "Phone access on" : "Phone access off");
      } catch (e) {
        toast(String(e));
        ev.target.checked = !enabled;
      } finally {
        ev.target.disabled = false;
      }
    });
    $("toggle-browser").addEventListener("change", (ev) => {
      invoke("set_open_browser", { enabled: ev.target.checked }).catch((e) => toast(String(e)));
    });
    for (const details of document.querySelectorAll("details.log")) {
      details.addEventListener("toggle", () => {
        if (details.open) loadLogs();
      });
    }
  }

  async function init() {
    wire();
    await listen("task-event", (ev) => {
      const p = ev.payload;
      if (p.kind === "step") onStep(p);
      else if (p.kind === "progress") onProgress(p);
      else if (p.kind === "finished") onFinished(p);
    });
    await listen("server-event", (ev) => {
      state.lastServerEvent = ev.payload;
      state.lastServerEventAt = Date.now();
      if (state.status) state.status.server = ev.payload;
      uiLog(`server-event: ${ev.payload.state}`);
      renderServer(ev.payload);
    });
    await listen("log-line", (ev) => appendLog(ev.payload));
    uiLog("listeners registered");
    await refreshStatus();
    // Keep the phone address fresh (Wi-Fi changes) and the status honest.
    setInterval(() => {
      if (!state.task) refreshStatus();
    }, 15000);
  }

  document.addEventListener("DOMContentLoaded", init);
})();
