<script lang="ts">
  import type { Hit } from "./api";

  let { hit }: { hit: Hit } = $props();
  let sources = $derived(hit.sources ?? (hit.project_id.startsWith("cf:") ? ["curseforge"] : ["modrinth"]));
</script>

<!-- Simple drawn marks (not the sites' logos) so it's clear where a project can come from. -->
<span class="srcs">
  {#each sources as s (s)}
    <span class="src {s}" title={s === "curseforge" ? "On CurseForge" : "On Modrinth"}>
      {#if s === "curseforge"}
        <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M2 5h9l3-2v3c0 2-1.5 3.5-3.5 4L11 13H5l.5-3C3.5 9.5 2 7.5 2 5Z" fill="currentColor" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="2" /><path d="M8 4.5v4l2.5 1.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
      {/if}
    </span>
  {/each}
</span>

<style>
  .srcs { display: inline-flex; gap: 3px; vertical-align: middle; }
  .src { display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 4px; }
  .src.curseforge { color: var(--cf); background: color-mix(in srgb, var(--cf) 16%, transparent); }
  .src.modrinth { color: var(--mr); background: color-mix(in srgb, var(--mr) 16%, transparent); }
</style>
