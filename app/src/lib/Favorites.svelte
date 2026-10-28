<script lang="ts">
  // A pack's Favorites tab: everything hearted anywhere, to talk over and then install here (or open another pack and
  // install it there). Removed shows what was un-hearted in the last day, newest first, so a slip can be undone.
  import { pushOverlay } from "./state.svelte";
  import { typeName } from "./api";
  import InstallButton from "./InstallButton.svelte";
  import { favorites, removedNewestFirst, toggleFavorite, type Fav } from "./favstore.svelte";

  let showRemoved = $state(false);
  let query = $state("");

  const since = (ms: number) => {
    const m = Math.max(0, Math.round((Date.now() - ms) / 60000));
    if (m < 1) return "just now";
    if (m < 60) return `${m} min ago`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h} h ago`;
    const d = Math.round(h / 24);
    return d === 1 ? "yesterday" : `${d} days ago`;
  };

  let list = $derived.by(() => {
    const all: Fav[] = showRemoved ? removedNewestFirst() : favorites.saved;
    const q = query.trim().toLowerCase();
    return q ? all.filter((f) => `${f.hit.title} ${f.hit.author}`.toLowerCase().includes(q)) : all;
  });
</script>

<div class="favs">
  <div class="bar">
    <div class="seg" role="tablist">
      <button role="tab" class:on={!showRemoved} onclick={() => (showRemoved = false)}>Saved ({favorites.saved.length})</button>
      <button role="tab" class:on={showRemoved} onclick={() => (showRemoved = true)}>Removed ({favorites.removed.length})</button>
    </div>
    <input class="search" type="search" placeholder="Search favorites" bind:value={query} />
  </div>

  <div class="list">
    {#each list as f (f.hit.project_id)}
      <div class="item">
        <button class="open" onclick={() => pushOverlay({ kind: "project", hit: f.hit })} title="Open its page">
          <span class="ic">{#if f.hit.icon_url}<img src={f.hit.icon_url} alt="" loading="lazy" />{/if}</span>
          <span class="txt"><strong>{f.hit.title}</strong><small>{f.hit.author ? `by ${f.hit.author} · ` : ""}{showRemoved ? `removed ${since(f.removedMs ?? 0)}` : `saved ${since(f.addedMs)}`}</small></span>
        </button>
        <span class="chip">{typeName(f.hit.project_type)}</span>
        {#if !showRemoved}<InstallButton hit={f.hit} />{/if}
        <button class="heart" class:on={!showRemoved} onclick={() => toggleFavorite(f.hit)}
          title={showRemoved ? "Put it back in favorites" : "Remove from favorites"} aria-label={showRemoved ? "Restore" : "Remove"}>{showRemoved ? "♡" : "♥"}</button>
      </div>
    {:else}
      <p class="empty">{query ? "Nothing matches your search." : showRemoved ? "Nothing removed in the last 24 hours." : "No favorites yet. Click the ♡ on a mod, resource pack or shader's page to save it here for later."}</p>
    {/each}
  </div>
</div>

<style>
  .favs { display: flex; flex-direction: column; gap: 12px; margin-top: 12px; }
  .bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .seg button { padding: 6px 12px; background: transparent; border: 0; color: inherit; }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button.on { background: var(--accent); color: #fff; font-weight: 600; }
  .search { flex: 1; min-width: 160px; max-width: 320px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 10px; font: inherit; }
  .list { display: flex; flex-direction: column; gap: 6px; }
  .item { display: flex; align-items: center; gap: 12px; min-width: 0; background: var(--panel); border-radius: 8px; padding: 8px 12px; }
  .open { display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0; background: none; border: 0; padding: 0; color: inherit; text-align: left; }
  .ic { width: 36px; height: 36px; flex: none; border-radius: 6px; overflow: hidden; background: var(--panel-2); }
  .ic img { width: 100%; height: 100%; object-fit: cover; }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .txt strong, .txt small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .txt small, .empty { color: var(--muted); }
  .chip { flex: none; font-size: 12px; padding: 2px 8px; border-radius: 999px; background: var(--panel-2); color: var(--text-2); }
  .heart { width: 34px; height: 34px; flex: none; border-radius: 8px; border: 1px solid var(--line); background: var(--panel); color: var(--muted); font-size: 18px; line-height: 1; }
  .heart:hover { color: var(--text); }
  .heart.on { color: #ff5c7a; border-color: color-mix(in srgb, #ff5c7a 50%, var(--line)); }
  @media (max-width: 700px) { .chip { display: none; } }
</style>
