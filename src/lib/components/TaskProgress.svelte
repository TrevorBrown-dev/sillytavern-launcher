<script lang="ts">
  import { app } from "../store.svelte";
  import { humanBytes } from "../format";
  import LogPanel from "./LogPanel.svelte";

  const GLYPH: Record<string, string> = { pending: "", running: "", done: "✓", skipped: "–", error: "!" };

  let isInstall = $derived(app.task === "install");
  let adopting = $derived(isInstall && !!app.lastInstallOptions.adoptPath);
  let title = $derived.by(() => {
    switch (app.task) {
      case "install":
        return adopting ? "Setting up your SillyTavern" : "Installing SillyTavern";
      case "update":
        return "Updating SillyTavern";
      case "selfupdate":
        return "Updating the launcher";
      default:
        return "";
    }
  });
  let subtitle = $derived.by(() => {
    switch (app.task) {
      case "install":
        return adopting
          ? "Nothing is copied or moved. The launcher checks the packages, adds the phone whitelist and sets up shortcuts."
          : "Grab a drink. Downloading and unpacking takes a few minutes, mostly on the Packages step.";
      case "update":
        return "Your chats, characters and settings are kept. This takes a few minutes.";
      case "selfupdate":
        return "The launcher restarts itself when this finishes. SillyTavern is stopped briefly.";
      default:
        return "";
    }
  });
  let percent = $derived(
    app.progress?.total ? Math.min(100, (app.progress.done / app.progress.total) * 100) : null,
  );
  let progressText = $derived.by(() => {
    const p = app.progress;
    if (!p) return "";
    if (p.total) {
      return p.unit === "bytes"
        ? `${humanBytes(p.done)} of ${humanBytes(p.total)}`
        : `${p.done.toLocaleString()} of ${p.total.toLocaleString()} ${p.unit}`;
    }
    return p.unit === "bytes" ? `${humanBytes(p.done)} downloaded` : String(p.done);
  });
</script>

<section>
  <h2>{title}</h2>
  <p class="muted">{subtitle}</p>

  <div class="card">
    <ol class="steps">
      {#each app.steps as step (step.id)}
        <li class="step" data-status={step.status}>
          <span class="icon">{GLYPH[step.status]}</span>
          <div>
            <span class="label">{step.label}</span>
            <span class="detail">{step.detail}</span>
          </div>
        </li>
      {/each}
    </ol>
    {#if app.progress}
      <div class="progress">
        <div class="track">
          <div
            class="bar"
            class:indeterminate={percent === null}
            style:width={percent === null ? undefined : `${percent.toFixed(1)}%`}
          ></div>
        </div>
        <div class="progress-text">{progressText}</div>
      </div>
    {/if}
  </div>

  {#if app.taskError}
    <div class="card error">
      <strong>Something went wrong.</strong>
      <pre>{app.taskError}</pre>
      <div class="actions">
        <button class="btn primary" onclick={() => app.retryTask()}>Try again</button>
        {#if !isInstall || app.status?.installed}
          <button class="btn" onclick={() => app.leaveTask()}>Back</button>
        {/if}
      </div>
    </div>
  {/if}

  <LogPanel label="Details" />
</section>

<style>
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .step {
    display: grid;
    grid-template-columns: 28px 1fr;
    gap: 12px;
    align-items: start;
    padding: 9px 6px;
    border-radius: 10px;
  }

  .step[data-status="running"] {
    background: rgba(240, 160, 75, 0.07);
  }

  .icon {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    border: 2px solid var(--border);
    display: grid;
    place-items: center;
    font-size: 12px;
    font-weight: 700;
    margin-top: 2px;
    color: transparent;
  }

  .step[data-status="done"] .icon {
    background: var(--ok);
    border-color: var(--ok);
    color: #0d2a16;
  }

  .step[data-status="skipped"] .icon {
    border-color: var(--muted);
    color: var(--muted);
  }

  .step[data-status="error"] .icon {
    background: var(--err);
    border-color: var(--err);
    color: #2a0d0d;
  }

  .step[data-status="running"] .icon {
    border-color: var(--accent);
    border-top-color: transparent;
    animation: spin 0.9s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .label {
    font-weight: 600;
  }

  .step[data-status="pending"] .label {
    color: var(--muted);
    font-weight: 500;
  }

  .detail {
    display: block;
    color: var(--muted);
    font-size: 13px;
  }

  .progress {
    margin-top: 14px;
  }

  .track {
    height: 8px;
    background: rgba(255, 255, 255, 0.07);
    border-radius: 999px;
    overflow: hidden;
  }

  .bar {
    height: 100%;
    width: 0%;
    background: linear-gradient(90deg, var(--accent), var(--accent-hover));
    border-radius: 999px;
    transition: width 0.15s linear;
  }

  .bar.indeterminate {
    width: 30%;
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      margin-left: 0;
    }
    50% {
      margin-left: 70%;
    }
    100% {
      margin-left: 0;
    }
  }

  .progress-text {
    margin-top: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }

  .error {
    margin-top: 16px;
    border-color: rgba(255, 114, 114, 0.5);
  }

  .error pre {
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--muted);
    font-size: 12.5px;
    margin: 8px 0 0;
    max-height: 160px;
    overflow: auto;
  }
</style>
