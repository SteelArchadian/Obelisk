<script lang="ts">
  // Project page → Relations. Opens on "Required by": the mods that won't run without this one (CurseForge lists them;
  // Modrinth doesn't, so a Modrinth project uses its CurseForge twin). Modpacks are left out unless asked for.
  import { invoke } from "@tauri-apps/api/core";
  import { untrack } from "svelte";
  import { compact, type Hit } from "./api";
  import { openProjectById } from "./install";

  let { hit, cfId, checking = false, deps = [] }: { hit: Hit; cfId: number | null; checking?: boolean; deps?: { project_id: string; dependency_type: string }[] } = $props();

  type Row = { id: string; name: string; author?: string; icon?: string; kind: string; cls?: string; downloads?: number };
  const KIND: Record<string, string> = {
    RequiredDependency: "Required", OptionalDependency: "Optional", Include: "Bundled in", EmbeddedLibrary: "Embedded",
    Tool: "Tool", Incompatible: "Incompatible", required: "Required", optional: "Optional", embedded: "Embedded", incompatible: "Incompatible",
  };

  type Mode = "required" | "optional" | "dependencies" | "all";
  const MODES: [Mode, string][] = [["required", "Required by"], ["optional", "Optional for"], ["dependencies", "Needs"], ["all", "All relations"]];
  let mode = $state<Mode>("required");
  let which = $derived<"dependencies" | "dependents">(mode === "dependencies" || !cfId ? "dependencies" : "dependents");
  let kind = $derived(mode === "required" ? "RequiredDependency" : mode === "optional" ? "OptionalDependency" : "");
  let hidePacks = $state(true);
  const SORTS = [["popular", "Most popular"], ["updated", "Recently updated"], ["newest", "Newest"], ["name", "Name A–Z"]] as const;
  let sort = $state<string>("popular");
  let sentinel = $state<HTMLElement>();
  let relEl = $state<HTMLElement>();
  let rows = $state<Row[]>([]);
  let total = $state(0);
  let page = $state(0);
  let busy = $state(false);
  let error = $state("");

  const cfRow = (x: any): Row => ({ id: `cf:${x.id}`, name: x.name, author: x.authorName, icon: x.logoUrl, kind: x.type, cls: x.categoryClass?.name, downloads: x.downloads });

  async function load(reset: boolean) {
    busy = true; error = "";
    if (reset) {
      rows = []; page = 0;
      // A new sort or type starts at the top, not at the end of the old list (which would load page after page).
      const box = relEl?.closest(".content");
      if (box && box.scrollTop > 0) box.scrollTop = 0;
    }
    try {
      if (cfId) {
        const r = await invoke<any>("cf_relations", { id: cfId, which, index: page * 20, kind, sort });
        rows = [...rows, ...(r.data ?? []).map(cfRow)];
        total = r.pagination?.totalCount ?? rows.length;
      } else if (which === "dependencies") {
        const r = await invoke<any>("mr_dependencies", { id: hit.project_id });
        const type = new Map(deps.map((d) => [d.project_id, d.dependency_type]));
        rows = (r.projects ?? []).map((p: any) => ({ id: p.id, name: p.title, icon: p.icon_url, kind: type.get(p.id) ?? "required", cls: p.project_type, downloads: p.downloads }));
        total = rows.length;
      }
    } catch (e) { error = String(e); }
    busy = false;
  }

  // Reload from the top only when the view changes (untracked: load itself reads the page and rows it changes).
  $effect(() => { void which; void kind; void cfId; void sort; if (!checking) untrack(() => load(true)); });

  // Scrolling near the end loads the next page by itself.
  const more = () => { if (!busy && cfId && rows.length < total && !error) { page += 1; load(false); } };
  const near = () => { if (sentinel && sentinel.getBoundingClientRect().top < innerHeight + 400) more(); };
  $effect(() => {
    if (!sentinel) return;
    untrack(near); // a short first page leaves the end already in view
    addEventListener("scroll", near, { capture: true, passive: true });
    return () => removeEventListener("scroll", near, { capture: true });
  });

  const isPack = (r: Row) => /modpack/i.test(r.cls ?? "") || r.kind === "Include";
  let shown = $derived(which === "dependents" && (hidePacks || mode !== "all") ? rows.filter((r) => !isPack(r)) : rows);
</script>

<div class="rel" bind:this={relEl}>
  <div class="bar">
    <div class="seg">
      {#each MODES.filter(([m]) => cfId || m === "dependencies") as [m, label] (m)}
        <button class:on={which === (m === "dependencies" ? "dependencies" : "dependents") && (mode === m || !cfId)} onclick={() => (mode = m)}>{label}</button>
      {/each}
    </div>
    {#if which === "dependents"}
      <select class="sort" bind:value={sort} aria-label="Sort">
        {#each SORTS as [v, l] (v)}<option value={v}>{l}</option>{/each}
      </select>
    {/if}
    {#if mode === "all" && cfId}<label class="check"><input type="checkbox" bind:checked={hidePacks} /> Hide modpacks</label>{/if}
    {#if which === "dependents" && total && !busy}<span class="muted small">{total >= 10000 ? "10,000+" : total.toLocaleString()} {mode === "required" ? "need this to run" : mode === "optional" ? "work with it if it's there" : "in all"}</span>{/if}
  </div>
  {#if checking}<p class="muted small">Looking for this project on CurseForge…</p>
  {:else if !cfId}<p class="muted small">This project isn't on CurseForge, and Modrinth doesn't list which mods need it, so only what it needs shows here.</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}

  <div class="list">
    {#each shown as r (r.id + r.kind)}
      <button class="item" onclick={() => openProjectById(r.id)}>
        <span class="ic">{#if r.icon}<img src={r.icon} alt="" loading="lazy" />{/if}</span>
        <span class="txt"><strong>{r.name}</strong>{#if r.author}<small>by {r.author}</small>{/if}</span>
        {#if r.cls}<span class="chip">{r.cls}</span>{/if}
        <span class="chip k">{KIND[r.kind] ?? r.kind}</span>
        {#if r.downloads}<small class="muted">⤓ {compact(r.downloads)}</small>{/if}
      </button>
    {:else}
      {#if !busy && !error}<p class="muted">{which === "dependents" && rows.length ? "Only modpacks use this on this page. Untick Hide modpacks, or load more." : "Nothing here."}</p>{/if}
    {/each}
  </div>
  {#if busy}<p class="muted">Loading…</p>{/if}
  {#if cfId && rows.length < total && !busy}<div bind:this={sentinel} class="sentinel"></div>{/if}
</div>

<style>
  .rel { display: flex; flex-direction: column; gap: 12px; }
  .bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .seg button { padding: 6px 12px; background: transparent; border: 0; color: inherit; }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button.on { background: var(--accent); color: #fff; font-weight: 600; }
  .check { display: flex; align-items: center; gap: 6px; }
  .list { display: flex; flex-direction: column; gap: 6px; }
  .item { display: flex; align-items: center; gap: 12px; min-width: 0; text-align: left; background: var(--panel); border: 1px solid transparent; border-radius: 8px; padding: 8px 12px; color: inherit; }
  .item:hover { border-color: var(--line); }
  .ic { width: 36px; height: 36px; flex: none; border-radius: 6px; overflow: hidden; background: var(--panel-2); }
  .ic img { width: 100%; height: 100%; object-fit: cover; }
  .txt { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .txt strong, .txt small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .txt small, .muted { color: var(--muted); }
  .chip { flex: none; font-size: 12px; padding: 2px 8px; border-radius: 999px; background: var(--panel-2); color: var(--text-2); }
  .chip.k { color: var(--accent); }
  .small { font-size: 13px; margin: 0; }
  .error { color: var(--err-text); background: var(--err-bg); padding: 8px 10px; border-radius: 6px; margin: 0; }
  .sort { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; font: inherit; }
  .sentinel { height: 1px; }
  @media (max-width: 700px) { .item .chip:not(.k), .item small.muted { display: none; } }
</style>
