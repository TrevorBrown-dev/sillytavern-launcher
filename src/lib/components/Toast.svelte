<script lang="ts">
  import { app } from "../store.svelte";
</script>

{#if app.toast}
  <div class="toast" role="status">{app.toast}</div>
{/if}

<style>
  .toast {
    position: fixed;
    left: 50%;
    bottom: 22px;
    transform: translateX(-50%);
    background: #2b2340;
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 10px 16px;
    font-size: 14px;
    box-shadow: var(--shadow);
    z-index: 10;
    max-width: 80vw;
  }
</style>
