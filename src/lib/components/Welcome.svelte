<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText, type ExistingInstall } from "../api";
  import { app } from "../store.svelte";

  let lan = $state(true);
  let shortcut = $state(true);
  let scanning = $state(true);
  let picking = $state(false);
  let found = $state<ExistingInstall[]>([]);
  let choice = $state<"adopt" | "fresh">("fresh");
  let selectedPath = $state<string | null>(null);

  let adopting = $derived(choice === "adopt" && selectedPath !== null);

  onMount(async () => {
    try {
      found = await api.discoverInstalls();
      if (found.length) {
        choice = "adopt";
        selectedPath = found[0].path;
      }
    } catch (e) {
      app.showToast(errorText(e));
    } finally {
      scanning = false;
    }
  });

  function describe(f: ExistingInstall): string {
    const parts = [`SillyTavern ${f.version ?? "(unknown version)"}`];
    if (f.isGit) parts.push(f.branch ? `git, ${f.branch} branch` : "git");
    if (f.port !== 8000) parts.push(`port ${f.port}`);
    return parts.join(" · ");
  }

  async function browse() {
    picking = true;
    try {
      const picked = await api.pickInstallFolder();
      if (!picked) return;
      if (!found.some((f) => f.path.toLowerCase() === picked.path.toLowerCase())) {
        found = [picked, ...found];
      }
      choice = "adopt";
      selectedPath = picked.path;
    } catch (e) {
      app.showToast(errorText(e), 6000);
    } finally {
      picking = false;
    }
  }

  function install() {
    void app.startInstall({
      lanEnabled: lan,
      createShortcut: shortcut,
      addFirewallRule: lan,
      adoptPath: adopting ? selectedPath : null,
    });
  }
</script>

<section>
  <div class="hero">
    <h2>Let's set up SillyTavern</h2>
    <p>
      This downloads SillyTavern and everything it needs into your own user folder. No terminal, no admin
      account, nothing else on your PC gets touched. It takes a few minutes.
    </p>
  </div>

  {#if scanning}
    <p class="tiny scanning">Looking for an existing SillyTavern on this PC...</p>
  {:else if found.length}
    <div class="card existing">
      <h3>SillyTavern is already on this PC</h3>
      <p class="muted small">
        The launcher can run it right where it is. Your chats, characters and settings stay put, nothing is
        copied, and your old way of starting it keeps working too.
      </p>
      {#each found as f (f.path)}
        <label class="choice">
          <input
            type="radio"
            name="source"
            checked={choice === "adopt" && selectedPath === f.path}
            onchange={() => {
              choice = "adopt";
              selectedPath = f.path;
            }}
          />
          <span class="text">
            <strong>Use <code>{f.path}</code></strong>
            <span>{describe(f)}</span>
          </span>
        </label>
      {/each}
      <label class="choice">
        <input type="radio" name="source" checked={choice === "fresh"} onchange={() => (choice = "fresh")} />
        <span class="text">
          <strong>Install a fresh copy instead</strong>
          <span>Into your user folder. The existing one is left untouched.</span>
        </span>
      </label>
      <p class="tiny">
        Somewhere else? <button class="link" onclick={browse} disabled={picking}>Choose its folder</button>
      </p>
    </div>
  {:else}
    <p class="tiny">
      Already have SillyTavern on this PC? <button class="link" onclick={browse} disabled={picking}>Choose its folder</button>
      and the launcher will run it in place.
    </p>
  {/if}

  <div class="card options">
    <label class="option">
      <input type="checkbox" bind:checked={lan} />
      <span class="text">
        <strong>Use it from my phone or tablet</strong>
        <span>
          Lets devices on your home Wi-Fi open SillyTavern. Windows will ask once for permission to allow it
          through the firewall.
        </span>
      </span>
    </label>
    <label class="option">
      <input type="checkbox" bind:checked={shortcut} />
      <span class="text">
        <strong>Add a desktop shortcut</strong>
        <span>Also adds SillyTavern to the Start Menu.</span>
      </span>
    </label>
  </div>

  {#if app.status}
    <p class="tiny install-to">
      {#if adopting}
        Runs from <code>{selectedPath}</code>; the launcher itself lives in <code>{app.status.home}</code>
      {:else}
        Installs to <code>{app.status.home}</code>
      {/if}
    </p>
  {/if}
  <button class="btn primary big" onclick={install} disabled={scanning}>
    {adopting ? "Use this SillyTavern" : "Install SillyTavern"}
  </button>
</section>

<style>
  .hero p {
    color: var(--muted);
    max-width: 640px;
    margin: 0 0 18px;
  }

  .scanning {
    margin: 0 0 16px;
  }

  .existing {
    margin-bottom: 16px;
  }

  .existing p.small {
    margin: 0 0 8px;
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .option,
  .choice {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    padding: 10px 8px;
    border-radius: 10px;
    cursor: pointer;
  }

  .option:hover,
  .choice:hover {
    background: rgba(255, 255, 255, 0.03);
  }

  .option input,
  .choice input {
    margin-top: 4px;
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
    flex-shrink: 0;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .text > span {
    color: var(--muted);
    font-size: 13.5px;
  }

  .text code {
    word-break: break-all;
  }

  .install-to {
    margin-top: 14px;
  }

  .big {
    margin-top: 18px;
  }
</style>
