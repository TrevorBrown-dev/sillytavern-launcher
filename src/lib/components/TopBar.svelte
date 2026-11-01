<script lang="ts">
  import { api } from "../api";
  import { app } from "../store.svelte";
</script>

<header>
  <div class="brand">
    <img src="/icon.png" alt="" width="36" height="36" />
    <div>
      <h1>SillyTavern</h1>
      <span class="sub">Launcher</span>
    </div>
  </div>
  <div class="right">
    {#if app.status}
      <span class="pill">Launcher {app.status.launcherVersion}</span>
    {/if}
    <button class="btn ghost small" title="Stops SillyTavern and closes the launcher" onclick={() => api.quit()}>
      Quit
    </button>
  </div>
</header>

<style>
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 28px 8px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .brand img {
    border-radius: 9px;
    box-shadow: var(--shadow);
  }

  h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 650;
    letter-spacing: 0.2px;
    line-height: 1.1;
  }

  .sub {
    font-size: 12px;
    color: var(--muted);
    letter-spacing: 1.5px;
    text-transform: uppercase;
  }

  .right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .pill {
    font-size: 12px;
    color: var(--muted);
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 3px 10px;
  }
</style>
