<script lang="ts">
  import { app } from "../store.svelte";
  import UpdateBanner from "./UpdateBanner.svelte";
  import StatusCard from "./StatusCard.svelte";
  import MaintenanceCard from "./MaintenanceCard.svelte";
  import PhoneCard from "./PhoneCard.svelte";
  import LogPanel from "./LogPanel.svelte";
</script>

{#if app.status}
  <section>
    <UpdateBanner />
    <div class="grid">
      <div class="column">
        <StatusCard server={app.status.server} />
        <MaintenanceCard status={app.status} />
      </div>
      <div class="column">
        <PhoneCard phone={app.status.phone} settings={app.status.settings} />
      </div>
    </div>
    <LogPanel label="Log" />
  </section>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: 1.05fr 1fr;
    gap: 16px;
    align-items: start;
  }

  @media (max-width: 860px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
</style>
