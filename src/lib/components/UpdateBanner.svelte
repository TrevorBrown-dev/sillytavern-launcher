<script lang="ts">
  import { app } from "../store.svelte";

  let st = $derived(app.updates?.sillytavern ?? null);
  let launcher = $derived(app.updates?.launcher ?? null);
  let show = $derived(!app.updatesDismissed && ((st?.available ?? false) || (launcher?.available ?? false)));

  let stText = $derived.by(() => {
    if (!st?.available) return null;
    return st.method === "git"
      ? `SillyTavern: ${st.detail ?? "new commits are available."}`
      : `SillyTavern ${st.latest} is available (you have ${st.installed ?? "an older version"}).`;
  });
</script>

{#if show}
  <div class="banner" role="status">
    <div class="text">
      {#if stText}<div>{stText}</div>{/if}
      {#if launcher?.available}
        <div>Launcher {launcher.latest} is available (you have {launcher.current}).</div>
      {/if}
    </div>
    <div class="actions">
      {#if st?.available}
        <button class="btn primary small" onclick={() => app.startUpdate()}>Update SillyTavern</button>
      {/if}
      {#if launcher?.available}
        <button class="btn primary small" onclick={() => app.startSelfUpdate()}>Update launcher</button>
      {/if}
      <button class="btn ghost small" onclick={() => (app.updatesDismissed = true)}>Later</button>
    </div>
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
    margin-bottom: 16px;
    padding: 12px 16px;
    border-radius: 12px;
    background: rgba(240, 160, 75, 0.1);
    border: 1px solid rgba(240, 160, 75, 0.45);
    font-size: 14px;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .actions {
    display: flex;
    gap: 8px;
    margin: 0;
  }
</style>
