<script lang="ts">
  /** The tile for a project with no picture of its own: the same outlined cube Modrinth shows for it. */
  let { size = "60%" }: { size?: string } = $props();
</script>

<span class="noicon" aria-hidden="true">
  <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
    <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
    <path d="M3.27 6.96 12 12.01l8.73-5.05M12 22.08V12" />
  </svg>
</span>

<style>
  .noicon { width: 100%; height: 100%; display: grid; place-items: center; background: var(--panel-2); color: var(--muted); }
</style>
