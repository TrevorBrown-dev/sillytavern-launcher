<script lang="ts">
  import { api, errorText, type StatusInfo, type UpdateCheck } from "../api";
  import { app } from "../store.svelte";

  let { status }: { status: StatusInfo } = $props();

  let checking = $state(false);
  let update = $state<UpdateCheck | null>(null);
  let updateError = $state<string | null>(null);

  async function checkUpdate() {
    checking = true;
    update = null;
    updateError = null;
    try {
      update = await api.checkUpdate();
    } catch (e) {
      updateError = errorText(e);
    } finally {
      checking = false;
    }
  }

  async function toastResult(action: () => Promise<string>) {
    try {
      app.showToast(await action());
    } catch (e) {
      app.showToast(errorText(e));
    }
  }

  function setOpenBrowser(ev: Event) {
    const enabled = (ev.currentTarget as HTMLInputElement).checked;
    api.setOpenBrowser(enabled).catch((e) => app.showToast(errorText(e)));
  }
</script>

<div class="card">
  <h3>Maintenance</h3>
  <p class="muted small">
    SillyTavern <strong>{status.stVersion ?? "?"}</strong> &middot; Node.js <strong>{status.nodeVersion ?? "?"}</strong>
  </p>
  <div class="actions">
    <button class="btn" disabled={checking} onclick={checkUpdate}>Check for updates</button>
    <button class="btn" onclick={() => api.openInstallFolder()}>Open install folder</button>
    <button class="btn" onclick={() => toastResult(api.createShortcuts)}>Re-create shortcuts</button>
  </div>

  {#if checking || update || updateError}
    <div class="update-box">
      {#if checking}
        <span>Checking...</span>
      {:else if updateError}
        <span>Could not check: {updateError}</span>
      {:else if update?.available}
        <span>Version {update.latest} is available (you have {update.installed ?? "an unknown version"}).</span>
        <button class="btn primary small" onclick={() => app.startUpdate()}>Update now</button>
      {:else if update}
        <span>You're on the latest version ({update.latest}).</span>
      {/if}
    </div>
  {/if}

  <label class="switch">
    <input type="checkbox" checked={status.settings.openBrowserOnStart} onchange={setOpenBrowser} />
    <span>Open the browser automatically when SillyTavern starts</span>
  </label>
</div>

<style>
  .update-box {
    margin-top: 12px;
    display: flex;
    align-items: center;
    gap: 12px;
    font-size: 13.5px;
    color: var(--muted);
  }
</style>
