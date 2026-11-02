<script lang="ts">
  // Right-click on anything marked data-link (a project's name, an author) offers to copy or open that link.
  import { toast, toastError } from "./state.svelte";
  import { openLink } from "./account.svelte";

  let menu = $state<{ url: string; label: string; x: number; y: number } | null>(null);

  function onContext(e: MouseEvent) {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-link]");
    if (!el?.dataset.link) return;
    e.preventDefault();
    menu = { url: el.dataset.link, label: el.dataset.linkLabel ?? "link", x: e.clientX, y: e.clientY };
  }

  async function copy(url: string, label: string) {
    menu = null;
    try {
      await navigator.clipboard.writeText(url);
      toast(`Copied the ${label}.`);
    } catch (e) { toastError(e); }
  }
</script>

<svelte:window oncontextmenu={onContext} onclick={() => (menu = null)} onkeydown={(e) => e.key === "Escape" && (menu = null)} onblur={() => (menu = null)} />

{#if menu}
  {@const m = menu}
  <div class="ctx" style:left="{Math.min(m.x, innerWidth - 220)}px" style:top="{Math.min(m.y, innerHeight - 100)}px" role="menu" tabindex="-1"
    onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
    <button onclick={() => copy(m.url, m.label)}>Copy {m.label}</button>
    <button onclick={() => { menu = null; openLink(m.url); }}>Open in browser</button>
  </div>
{/if}

<style>
  .ctx { position: fixed; z-index: 60; min-width: 200px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 5px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.4); display: flex; flex-direction: column; }
  .ctx button { font: inherit; color: inherit; cursor: pointer; text-align: left; background: none; border: 0; padding: 8px 12px; border-radius: 5px; }
  .ctx button:hover { background: var(--panel); }
</style>
