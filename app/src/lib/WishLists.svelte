<script lang="ts">
  // A collaborative pack's Wish lists tab: what each person would like added, to talk over before anyone installs
  // it. Everyone sees everyone's lists; only your own can be changed (the star on a project page, or here).
  import { pushOverlay } from "./state.svelte";
  import { typeName } from "./api";
  import InstallButton from "./InstallButton.svelte";
  import { collab, loadCollab, toggleWish, type Wish } from "./collab.svelte";

  let { instId }: { instId: string } = $props();

  $effect(() => { loadCollab(instId); });

  let c = $derived(collab[instId]);
  let who = $state<string>("all");
  let query = $state("");

  const nameOf = (id: string) => (id === c?.me ? "You" : c?.members?.find((m) => m.id === id)?.name || c?.wishes?.[id]?.name || "Friend");
  // People with a list, you first, then by name.
  let people = $derived(Object.entries(c?.wishes ?? {}).filter(([, w]) => w.items.length)
    .sort(([a], [b]) => (a === c?.me ? -1 : b === c?.me ? 1 : nameOf(a).localeCompare(nameOf(b)))));

  // "Everyone" merges the lists: one row per project, with everyone who wished for it.
  let rows = $derived.by(() => {
    const out = new Map<string, { wish: Wish; by: string[] }>();
    for (const [id, w] of people) {
      if (who !== "all" && who !== id) continue;
      for (const wish of w.items) {
        const r = out.get(wish.hit.project_id);
        if (r) { r.by.push(id); if (wish.addedMs > r.wish.addedMs) r.wish = wish; }
        else out.set(wish.hit.project_id, { wish, by: [id] });
      }
    }
    const q = query.trim().toLowerCase();
    return [...out.values()].filter((r) => !q || `${r.wish.hit.title} ${r.wish.hit.author}`.toLowerCase().includes(q))
      .sort((a, b) => b.by.length - a.by.length || b.wish.addedMs - a.wish.addedMs);
  });
  let total = $derived(new Set(people.flatMap(([, w]) => w.items.map((i) => i.hit.project_id))).size);
</script>

<div class="wish">
  <div class="bar">
    <div class="seg" role="tablist">
      <button role="tab" class:on={who === "all"} onclick={() => (who = "all")}>Everyone ({total})</button>
      {#each people as [id, w] (id)}
        <button role="tab" class:on={who === id} onclick={() => (who = id)}>{nameOf(id)} ({w.items.length})</button>
      {/each}
    </div>
    <input class="search" type="search" placeholder="Search wish lists" bind:value={query} />
  </div>

  <div class="list">
    {#each rows as r (r.wish.hit.project_id)}
      {@const mine = !!c?.me && r.by.includes(c.me)}
      <div class="item">
        <button class="open" onclick={() => pushOverlay({ kind: "project", hit: r.wish.hit })} title="Open its page">
          <span class="ic">{#if r.wish.hit.icon_url}<img src={r.wish.hit.icon_url} alt="" loading="lazy" />{/if}</span>
          <span class="txt"><strong>{r.wish.hit.title}</strong><small>Wished for by {r.by.map(nameOf).join(", ")}</small></span>
        </button>
        <span class="chip">{typeName(r.wish.hit.project_type)}</span>
        <InstallButton hit={r.wish.hit} />
        <button class="star" class:on={mine} onclick={() => toggleWish(instId, r.wish.hit)}
          title={mine ? "Take it off your wish list" : "Add it to your wish list too"} aria-label={mine ? "Take off your wish list" : "Add to your wish list"}>{mine ? "★" : "☆"}</button>
      </div>
    {:else}
      <p class="empty">{query ? "Nothing matches your search." : "No wishes yet. While adding content to this pack, click the ☆ on a project's page to wish-list it for everyone to see."}</p>
    {/each}
  </div>
</div>

<style>
  .wish { display: flex; flex-direction: column; gap: 12px; margin-top: 12px; }
  .bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .seg { display: inline-flex; flex-wrap: wrap; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
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
  .star { width: 34px; height: 34px; flex: none; border-radius: 8px; border: 1px solid var(--line); background: var(--panel); color: var(--muted); font-size: 18px; line-height: 1; }
  .star:hover { color: var(--text); }
  .star.on { color: #f5c542; border-color: color-mix(in srgb, #f5c542 50%, var(--line)); }
  @media (max-width: 700px) { .chip { display: none; } }
</style>
