<script lang="ts">
  import type { Hit } from "./api";
  import { openLink } from "./account.svelte";

  // links: each site's page for this project; a badge with one is a button that opens it.
  let { hit, big = false, links = {} }: { hit: Hit; big?: boolean; links?: Partial<Record<string, string | null>> } = $props();
  let sources = $derived(hit.sources ?? (hit.project_id.startsWith("cf:") ? ["curseforge"] : ["modrinth"]));
</script>

<!-- Simple drawn marks (not the sites' logos) so it's clear where a project can come from. -->
<span class="srcs" class:big>
  {#each sources as s (s)}
    <svelte:element this={links[s] ? "button" : "span"} class="src {s}" class:link={!!links[s]} role={links[s] ? undefined : "img"}
      title={links[s] ? `Open on ${s === "curseforge" ? "CurseForge" : "Modrinth"}` : s === "curseforge" ? "On CurseForge" : "On Modrinth"}
      onclick={links[s] ? () => openLink(links[s]) : undefined}>
      {#if s === "curseforge"}
        <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M2 5h9l3-2v3c0 2-1.5 3.5-3.5 4L11 13H5l.5-3C3.5 9.5 2 7.5 2 5Z" fill="currentColor" /></svg>
      {:else}
        <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><circle cx="8" cy="8" r="5.5" fill="none" stroke="currentColor" stroke-width="2" /><path d="M8 4.5v4l2.5 1.5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
      {/if}
      {#if big}<span class="name">{s === "curseforge" ? "CurseForge" : "Modrinth"}{#if links[s]}<span class="arrow">↗</span>{/if}</span>{/if}
    </svelte:element>
  {/each}
</span>

<style>
  .srcs { display: inline-flex; gap: 3px; vertical-align: middle; }
  .src { display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 4px; }
  .big { gap: 6px; }
  .big .src { width: auto; height: 26px; padding: 0 9px 0 7px; gap: 6px; display: inline-flex; align-items: center; border-radius: 6px; font-size: 12.5px; font-weight: 600; }
  .big .src svg { width: 15px; height: 15px; }
  .src.curseforge { color: var(--cf); background: color-mix(in srgb, var(--cf) 16%, transparent); }
  .src.modrinth { color: var(--mr); background: color-mix(in srgb, var(--mr) 16%, transparent); }
  .src.link { border: 0; font: inherit; cursor: pointer; }
  .arrow { margin-left: 4px; }
  .src.link:hover { filter: brightness(1.25); }
</style>
