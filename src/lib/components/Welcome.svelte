<script lang="ts">
  import { app } from "../store.svelte";

  let lan = $state(true);
  let shortcut = $state(true);

  function install() {
    void app.startInstall({ lanEnabled: lan, createShortcut: shortcut, addFirewallRule: lan });
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
    <p class="tiny install-to">Installs to <code>{app.status.home}</code></p>
  {/if}
  <button class="btn primary big" onclick={install}>Install SillyTavern</button>
</section>

<style>
  .hero p {
    color: var(--muted);
    max-width: 640px;
    margin: 0 0 18px;
  }

  .options {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .option {
    display: flex;
    gap: 14px;
    align-items: flex-start;
    padding: 10px 8px;
    border-radius: 10px;
    cursor: pointer;
  }

  .option:hover {
    background: rgba(255, 255, 255, 0.03);
  }

  .option input {
    margin-top: 4px;
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .text > span {
    color: var(--muted);
    font-size: 13.5px;
  }

  .install-to {
    margin-top: 14px;
  }

  .big {
    margin-top: 18px;
  }
</style>
