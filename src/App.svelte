<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/store.svelte";
  import TopBar from "./lib/components/TopBar.svelte";
  import Welcome from "./lib/components/Welcome.svelte";
  import TaskProgress from "./lib/components/TaskProgress.svelte";
  import Home from "./lib/components/Home.svelte";
  import Toast from "./lib/components/Toast.svelte";

  onMount(() => {
    void app.init();
  });
</script>

<div class="shell">
  <TopBar />
  <main>
    {#if app.screen === "welcome"}
      <Welcome />
    {:else if app.screen === "task"}
      <TaskProgress />
    {:else if app.screen === "home"}
      <Home />
    {/if}
  </main>
  <Toast />
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }

  main {
    flex: 1;
    overflow-y: auto;
    padding: 8px 28px 28px;
  }

  main > :global(*) {
    max-width: 960px;
    margin: 0 auto;
  }
</style>
