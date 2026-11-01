<script lang="ts">
  import { app } from "../store.svelte";

  let { label = "Log" }: { label?: string } = $props();

  let open = $state(false);
  let box = $state<HTMLPreElement | undefined>();

  $effect(() => {
    if (open) void app.loadLogs();
  });

  // Follow new lines while the panel is open.
  $effect(() => {
    void app.logLines.length;
    if (open && box) box.scrollTop = box.scrollHeight;
  });
</script>

<details bind:open>
  <summary>{label}</summary>
  <pre bind:this={box}>{app.logLines.join("\n")}</pre>
</details>

<style>
  details {
    margin-top: 16px;
  }

  summary {
    cursor: pointer;
    color: var(--muted);
    font-size: 13px;
    user-select: none;
  }

  pre {
    margin: 8px 0 0;
    background: #0f0c15;
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px 12px;
    font-family: var(--mono);
    font-size: 12px;
    line-height: 1.4;
    color: #c9c1da;
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
