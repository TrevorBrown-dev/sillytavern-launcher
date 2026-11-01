<script lang="ts">
  import { api, errorText, type StatusInfo } from "../api";
  import { app } from "../store.svelte";

  let { status }: { status: StatusInfo } = $props();

  let picking = $state(false);
  let startWithWindows = $state(false);
  $effect(() => {
    startWithWindows = status.startWithWindows;
  });

  let locationNote = $derived.by(() => {
    if (status.mode === "adopted") {
      return status.isGit
        ? `Your own install, run in place (git${status.branch ? `, ${status.branch} branch` : ""})`
        : "Your own install, run in place";
    }
    return "Installed by the launcher";
  });

  let stUpdateText = $derived.by(() => {
    const u = app.updates?.sillytavern;
    if (!u) return null;
    if (u.method === "git") return u.detail ?? (u.available ? "New commits are available." : "Up to date.");
    return u.available
      ? `Version ${u.latest} is available (you have ${u.installed ?? "an unknown version"}).`
      : `You're on the latest version (${u.latest}).`;
  });

  async function toastResult(action: () => Promise<string>) {
    try {
      app.showToast(await action());
    } catch (e) {
      app.showToast(errorText(e));
    }
  }

  function checked(ev: Event): boolean {
    return (ev.currentTarget as HTMLInputElement).checked;
  }

  function setOpenBrowser(ev: Event) {
    api.setOpenBrowser(checked(ev)).catch((e) => app.showToast(errorText(e)));
  }

  function setCheckUpdates(ev: Event) {
    api.setCheckUpdatesOnStart(checked(ev)).catch((e) => app.showToast(errorText(e)));
  }

  function setKeepAwake(ev: Event) {
    api.setKeepAwake(checked(ev)).catch((e) => app.showToast(errorText(e)));
  }

  async function setStartWithWindows(ev: Event) {
    const input = ev.currentTarget as HTMLInputElement;
    const enabled = input.checked;
    try {
      await api.setStartWithWindows(enabled);
      startWithWindows = enabled;
      app.showToast(enabled ? "SillyTavern will start with Windows (in the tray)" : "No longer starts with Windows");
    } catch (e) {
      app.showToast(errorText(e));
      input.checked = !enabled;
    }
  }

  async function useDifferentFolder() {
    picking = true;
    try {
      const picked = await api.pickInstallFolder();
      if (!picked) return;
      void app.startInstall({
        lanEnabled: status.settings.lanEnabled,
        createShortcut: true,
        addFirewallRule: status.settings.lanEnabled,
        adoptPath: picked.path,
      });
    } catch (e) {
      app.showToast(errorText(e), 6000);
    } finally {
      picking = false;
    }
  }
</script>

<div class="card">
  <h3>Maintenance</h3>
  <p class="muted small">
    SillyTavern <strong>{status.stVersion ?? "?"}</strong> &middot; Node.js <strong>{status.nodeVersion ?? "?"}</strong>
    &middot; Launcher <strong>{status.launcherVersion}</strong>
  </p>
  <p class="muted small location">
    <code>{status.stDir}</code><br />
    {locationNote}
  </p>
  <div class="actions">
    <button class="btn" disabled={app.checkingUpdates} onclick={() => app.checkUpdates()}>
      {app.checkingUpdates ? "Checking..." : "Check for updates"}
    </button>
    <button class="btn" onclick={() => app.repairInstall()} title="Re-runs setup; fixes a half-finished or broken install without losing anything">
      Repair installation
    </button>
    <button class="btn" onclick={() => api.openInstallFolder()}>Open SillyTavern folder</button>
    <button class="btn" onclick={() => api.openConfigFile().catch((e) => app.showToast(errorText(e)))}>Edit config.yaml</button>
    <button class="btn" onclick={() => toastResult(api.createShortcuts)}>Re-create shortcuts</button>
    <button class="btn" disabled={picking} onclick={useDifferentFolder}>Use a different SillyTavern folder...</button>
  </div>

  {#if app.updates}
    <div class="update-box">
      {#if stUpdateText}
        <div class="row">
          <span>SillyTavern: {stUpdateText}</span>
          {#if app.updates.sillytavern?.available}
            <button class="btn primary small" onclick={() => app.startUpdate()}>Update now</button>
          {/if}
        </div>
      {/if}
      {#if app.updates.launcher}
        <div class="row">
          <span>
            {#if app.updates.launcher.available}
              Launcher {app.updates.launcher.latest} is available (you have {app.updates.launcher.current}).
            {:else}
              Launcher: you're on the latest version ({app.updates.launcher.current}).
            {/if}
          </span>
          {#if app.updates.launcher.available}
            <button class="btn primary small" onclick={() => app.startSelfUpdate()}>Update launcher</button>
          {/if}
        </div>
      {/if}
      {#each app.updates.errors as err (err)}
        <div class="row error">Could not check: {err}</div>
      {/each}
    </div>
  {/if}

  <label class="switch">
    <input type="checkbox" checked={status.settings.openBrowserOnStart} onchange={setOpenBrowser} />
    <span>Open the browser automatically when SillyTavern starts</span>
  </label>
  <label class="switch">
    <input type="checkbox" checked={status.settings.keepAwake} onchange={setKeepAwake} />
    <span>Keep this PC awake while SillyTavern is running <em class="muted">(the screen can still turn off)</em></span>
  </label>
  <label class="switch">
    <input type="checkbox" checked={startWithWindows} onchange={setStartWithWindows} />
    <span>Start SillyTavern when I sign in to Windows <em class="muted">(in the tray, no window)</em></span>
  </label>
  <label class="switch">
    <input type="checkbox" checked={status.settings.checkUpdatesOnStart} onchange={setCheckUpdates} />
    <span>Check for updates when the launcher starts</span>
  </label>
</div>

<style>
  .location {
    margin: -4px 0 0;
    line-height: 1.5;
  }

  .location code {
    word-break: break-all;
  }

  .update-box {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 13.5px;
    color: var(--muted);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  .row.error {
    color: var(--err);
  }

  .switch + .switch {
    margin-top: 10px;
    padding-top: 10px;
  }

  .switch em {
    font-style: normal;
    font-size: 12.5px;
  }
</style>
