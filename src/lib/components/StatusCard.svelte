<script lang="ts">
  import { api, errorText, type ServerStatus } from "../api";
  import { app } from "../store.svelte";

  let { server }: { server: ServerStatus } = $props();

  const TITLES: Record<ServerStatus["state"], string> = {
    stopped: "Stopped",
    starting: "Starting up...",
    running: "Running",
    error: "Problem",
  };

  let subtitle = $derived.by(() => {
    switch (server.state) {
      case "stopped":
        return "SillyTavern is not running.";
      case "starting":
        return "Waiting for SillyTavern to answer. First start can take a minute.";
      case "running":
        return server.external ? `Already running at ${server.url}` : `Open at ${server.url}`;
      case "error":
        return "SillyTavern could not start.";
    }
  });

  async function run(action: () => Promise<unknown>) {
    try {
      await action();
    } catch (e) {
      app.showToast(errorText(e));
    }
  }
</script>

<div class="card">
  <div class="row">
    <span class="dot {server.state}"></span>
    <div class="text">
      <strong>{TITLES[server.state]}</strong>
      <span class="muted">{subtitle}</span>
    </div>
  </div>
  {#if server.state === "error" && server.message}
    <pre class="message">{server.message}</pre>
  {/if}
  <div class="actions">
    <button class="btn primary" disabled={server.state !== "running"} onclick={() => run(api.openBrowser)}>
      Open SillyTavern
    </button>
    <button
      class="btn"
      disabled={server.state === "running" || server.state === "starting"}
      onclick={() => run(api.startServer)}
    >
      Start
    </button>
    <button
      class="btn"
      disabled={server.state === "stopped" || (server.external && server.state === "running")}
      onclick={() => run(api.stopServer)}
    >
      Stop
    </button>
  </div>
  <p class="tiny">
    Closing this window keeps SillyTavern running in the tray (bottom-right by the clock). Use Quit to stop it.
  </p>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .text {
    display: flex;
    flex-direction: column;
  }

  .text strong {
    font-size: 17px;
  }

  .dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--muted);
    box-shadow: 0 0 0 4px rgba(255, 255, 255, 0.05);
    flex-shrink: 0;
  }

  .dot.running {
    background: var(--ok);
    box-shadow: 0 0 0 4px rgba(111, 208, 140, 0.18);
  }

  .dot.starting {
    background: var(--warn);
    box-shadow: 0 0 0 4px rgba(245, 196, 81, 0.18);
    animation: pulse 1s ease-in-out infinite;
  }

  .dot.error {
    background: var(--err);
    box-shadow: 0 0 0 4px rgba(255, 114, 114, 0.18);
  }

  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }

  .message {
    margin: 10px 0 0;
    white-space: pre-wrap;
    word-break: break-word;
    font-size: 12.5px;
    color: var(--err);
    max-height: 140px;
    overflow: auto;
  }
</style>
