<script lang="ts">
  // Small blocky icons for tabs, nav and stat cards (drawn on a 16px grid so they stay crisp).
  let { name, size = 16 }: { name: string; size?: number } = $props();
</script>

<svg class="icon" viewBox="0 0 16 16" width={size} height={size} aria-hidden="true" shape-rendering="crispEdges">
  {#if name === "cube"}
    <path d="M8 1 15 4.5v7L8 15 1 11.5v-7z" fill="currentColor" opacity="0.35" /><path d="M8 1 15 4.5 8 8 1 4.5z" fill="currentColor" /><path d="M8 8v7l7-3.5v-7z" fill="currentColor" opacity="0.65" />
  {:else if name === "compass"}
    <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm0 2a5 5 0 1 1 0 10A5 5 0 0 1 8 3z" fill="currentColor" /><path d="M10.5 5.5 9 9 5.5 10.5 7 7z" fill="currentColor" />
  {:else if name === "chest"}
    <path d="M1 4h14v3H1z" fill="currentColor" /><path d="M1 8h14v6H1z" fill="currentColor" opacity="0.65" /><path d="M7 6h2v4H7z" fill="var(--bg)" />
  {:else if name === "page"}
    <path d="M3 1h7l3 3v11H3z" fill="currentColor" opacity="0.4" /><path d="M5 6h6v1H5zm0 3h6v1H5zm0 3h4v1H5z" fill="currentColor" />
  {:else if name === "pulse"}
    <path d="M1 8h3l2-5 3 10 2-5h4" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" shape-rendering="auto" />
  {:else if name === "bars"}
    <path d="M2 10h3v5H2zm4.5-4h3v9h-3zM11 2h3v13h-3z" fill="currentColor" />
  {:else if name === "timer"}
    <path d="M6 0h4v2H6z" fill="currentColor" /><path d="M8 3a6 6 0 1 0 0 12A6 6 0 0 0 8 3zm0 2a4 4 0 1 1 0 8 4 4 0 0 1 0-8z" fill="currentColor" /><path d="M7.5 6h1.5v3.5H7.5z" fill="currentColor" />
  {:else if name === "tick"}
    <path d="M8 1a7 7 0 1 0 0 14A7 7 0 0 0 8 1zm0 2a5 5 0 1 1 0 10A5 5 0 0 1 8 3z" fill="currentColor" /><path d="M7 4h2v4h3v2H7z" fill="currentColor" />
  {:else if name === "memory"}
    <path d="M2 4h12v8H2z" fill="currentColor" opacity="0.45" /><path d="M4 6h2v4H4zm3 0h2v4H7zm3 0h2v4h-2zM3 12h1v2H3zm3 0h1v2H6zm3 0h1v2H9zm3 0h1v2h-1z" fill="currentColor" />
  {/if}
</svg>

<style>
  .icon { flex: none; vertical-align: -3px; }
</style>
