<script lang="ts">
  // Project page → Relations: what this project needs, and (CurseForge only) what uses it. Modpacks are hidden
  // from the dependents by default: most people look here for the mods built on this one.
  import { invoke } from "@tauri-apps/api/core";
  import { compact, type Hit } from "./api";
  import { openProjectById } from "./install";

  let { hit, cfId, deps = [] }: { hit: Hit; cfId: number | null; deps?: { project_id: string; dependency_type: string }[] } = $props();

  type Row = { id: string; name: string; author?: string; icon?: string; kind: string; cls?: string; downloads?: number };
  const KIND: Record<string, string> = {
    RequiredDependency: "Required", OptionalDependency: "Optional", Include: "Bundled in", EmbeddedLibrary: "Embedded",
    Tool: "Tool", Incompatible: "Incompatible", required: "Required", optional: "Optional", embedded: "Embedded", incompatible: "Incompatible",
  };

  let which = $state<"dependencies" | "dependents">("dependencies");
  let kind = $state("");
  let hidePacks = $state(true);
  let rows = $state<Row[]>([]);
  let total = $state(0);
  let page = $state(0);
  let busy = $state(false);
  let error = $state("");

  const cfRow = (x: any): Row => ({ id: `cf:${x.id}`, name: x.name, author: x.authorName, icon: x.logoUrl, kind: x.type, cls: x.categoryClass?.name, downloads: x.downloads });

  async function load(reset: boolean) {
    busy = true; error = "";
    if (reset) { rows = []; page = 0; }
    try {
      if (cfId) {
        const r = await invoke<any>("cf_relations", { id: cfId, which, index: page * 20, kind });
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

  $effect(() => { void which; void kind; void cfId; load(true); });

  const isPack = (r: Row) => /modpack/i.test(r.cls ?? "") || r.kind === "Include";
  let shown = $derived(which === "dependents" && hidePacks ? rows.filter((r) => !isPack(r)) : rows);
  const kinds = $derived(which === "dependents" ? ["", "RequiredDependency", "OptionalDependency", "Include"] : []);
</script>

<div class="rel">
  <div class="bar">
    <div class="seg">
      <button class:on={which === "dependencies"} onclick={() => { which = "dependencies"; kind = ""; }}>Dependencies</button>
      {#if cfId}<button class:on={which === "dependents"} onclick={() => { which = "dependents"; kind = ""; }}>Dependents</button>{/if}
    </div>
    {#if which === "dependents"}
      <select bind:value={kind} aria-label="Relation type">
        {#each kinds as k (k)}<option value={k}>{k ? KIND[k] : "All types"}</option>{/each}
      </select>
      <label class="check"><input type="checkbox" bind:checked={hidePacks} /> Hide modpacks</label>
    {/if}
  </div>
  {#if !cfId && which === "dependencies"}<p class="muted small">Modrinth doesn't list which projects depend on this one, so only its dependencies show here.</p>{/if}
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
  {#if busy}<p class="muted">Loading…</p>{:else if cfId && rows.length < total}
    <button class="ghost more" onclick={() => { page += 1; load(false); }}>Load more ({rows.length} of {total >= 10000 ? "10,000+" : total.toLocaleString()})</button>
  {/if}
</div>

<style>
  .rel { display: flex; flex-direction: column; gap: 12px; }
  .bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .seg button { padding: 6px 12px; background: transparent; border: 0; color: inherit; }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button.on { background: var(--accent); color: #fff; font-weight: 600; }
  select { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; font: inherit; }
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
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; color: inherit; }
  .more { align-self: flex-start; }
  @media (max-width: 700px) { .item .chip:not(.k), .item small.muted { display: none; } }
</style>
