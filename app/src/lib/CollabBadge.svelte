<script lang="ts">
  // Handshake in the top-left corner of a pack's picture when the pack is collaborative (pixel art, drawn at 2× so it stays sharp).
  import { app } from "./state.svelte";

  let { id }: { id: string } = $props();

  const ART = [
    "##.....#####..##",
    "##....#.....####",
    "##.###..###...##",
    "##....##...#..##",
    "##.....##...#.##",
    "##......##...###",
    "..#......##.#...",
    "...#.#.#..##....",
    "....#.#.#.#.....",
    ".....#####......",
  ];
  const pixels = ART.flatMap((row, y) => [...row].flatMap((c, x) => (c === "#" ? [[x, y]] : [])));
</script>

{#if id in app.shareIncoming}
  <span class="collab" title="Collaborative pack" aria-label="Collaborative pack">
    <svg viewBox="0 0 16 10" width="32" height="20" shape-rendering="crispEdges" fill="currentColor" aria-hidden="true">
      {#each pixels as [x, y] (`${x},${y}`)}<rect {x} {y} width="1" height="1" />{/each}
    </svg>
  </span>
{/if}

<style>
  .collab { position: absolute; top: 8px; left: 8px; z-index: 1; display: grid; place-items: center; color: #fff; background: rgb(0 0 0 / 0.65); padding: 2px 4px; border-radius: 4px; pointer-events: none; }
</style>
