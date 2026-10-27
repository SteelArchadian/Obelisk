<script lang="ts">
  import { settings } from "./settings.svelte";
  import { onMount, tick } from "svelte";
  import { api, compact, ago, envLabel, versionsLabel, TYPES, type Hit, type Tags, type ProjectType } from "./api";
  import { app, pushOverlay } from "./state.svelte";
  import { tagsOnce, loadersOf as loaderOf, cap, isInstalled } from "./install";
  import { cfSearchAvailable, cfToHit, mergeHits } from "./cf";
  import SourceBadges from "./SourceBadges.svelte";
  import InstallButton from "./InstallButton.svelte";
  import TargetChip from "./TargetChip.svelte";

  const SORTS = [
    { id: "relevance", label: "Relevancy" },
    { id: "downloads", label: "Most downloaded" },
    { id: "follows", label: "Most followed" },
    { id: "updated", label: "Latest update" },
    { id: "newest", label: "Release date" },
  ];

  let tags = $state<Tags | null>(null);
  let sort = $state(localStorage.getItem("obelisk.browseSort") ?? "relevance");
  let view = $state<"list" | "tiles" | "table">((localStorage.getItem("obelisk.browseView") as any) ?? "list");
  let tile = $state(Number(localStorage.getItem("obelisk.browseTile") ?? 180));
  // Any of these loaders (none ticked = any); several at once suits Sinytra Connector (NeoForge + Fabric).
  let loaders = $state<string[]>(app.target?.loader ? [app.target.loader.toLowerCase()] : []);
  let gameVersion = $state(app.target?.minecraft ?? "");
  let categories = $state<Set<string>>(new Set());
  let hideInstalled = $state(false);
  let hideLibraries = $state(false);
  // Modrinth files libraries under "library", CurseForge under "library-api" (API and Library).
  const isLibrary = (h: Hit) => h.categories.some((c) => c === "library" || c === "library-api");
  let showFilters = $state(false);
  let showSort = $state(false);

  let hits = $state<Hit[]>([]);
  let total = $state(0);
  /** Where results come from: both sites, or just one. */
  let source = $state<"all" | "modrinth" | "curseforge">((localStorage.getItem("obelisk.browseSource") as any) ?? "all");
  let cfOk = $state<boolean | null>(null);
  let hasKey = $state(false);
  let mrOffset = 0, mrTotal = 0, cfOffset = 0, cfTotal = 0;
  let useCf = $derived(source !== "modrinth" && cfOk === true && categories.size === 0);
  let useMr = $derived(source !== "curseforge" || cfOk === false);
  let loading = $state(false);
  let error = $state("");
  let sentinel = $state<HTMLElement>();
  let generation = 0;

  let typeLabel = $derived(TYPES.find((t) => t.type === app.browseType)?.label ?? "");
  let loaderApplies = $derived(app.browseType === "mod" || app.browseType === "modpack");
  let typeCategories = $derived(
    (tags?.categories ?? []).filter((c) => c.project_type === app.browseType && c.header !== "resolutions"),
  );
  let loaderNames = $derived(new Set((tags?.loaders ?? []).map((l) => l.name)));
  let shown = $derived(hits.filter((h) => !(hideInstalled && isInstalled(h)) && !(hideLibraries && isLibrary(h))));

  onMount(() => {
    tagsOnce().then((t) => (tags = t)).catch(() => {});
    cfSearchAvailable().then((ok) => (cfOk = ok));
    api.hasCfKey().then((k) => (hasKey = k));
    const io = new IntersectionObserver((e) => {
      if (e[0].isIntersecting && !loading && more()) load(false);
    }, { rootMargin: `${AHEAD}px` });
    if (sentinel) io.observe(sentinel);
    return () => io.disconnect();
  });

  // Re-run the search whenever anything that shapes it changes (debounced for typing).
  $effect(() => {
    const key = [app.browseType, app.browseQuery, sort, loaders.join(","), settings.connector, gameVersion, [...categories].join(","), useCf, useMr];
    void key;
    const t = setTimeout(() => load(true), 250);
    return () => clearTimeout(t);
  });

  $effect(() => {
    try {
      localStorage.setItem("obelisk.browseSort", sort);
      localStorage.setItem("obelisk.browseSource", source);
      localStorage.setItem("obelisk.browseView", view);
      localStorage.setItem("obelisk.browseTile", String(tile));
    } catch { /* ignore */ }
  });

  /** Start fetching the next page while it is still this far below the screen. */
  const AHEAD = 2400;
  const more = () => mrOffset < mrTotal || cfOffset < cfTotal;
  /** After a page lands the end may still be in range; the observer only fires on changes, so keep going. */
  async function topUp() {
    await tick();
    const r = sentinel?.getBoundingClientRect();
    if (r && r.top < innerHeight + AHEAD && !loading && !error && more()) load(false);
  }

  async function load(reset: boolean) {
    const gen = reset ? ++generation : generation;
    loading = true;
    error = "";
    try {
      if (reset) mrOffset = mrTotal = cfOffset = cfTotal = 0;
      const wantMr = useMr && (reset || mrOffset < mrTotal);
      const wantCf = useCf && (reset || cfOffset < cfTotal);
      // "Offer Fabric mods on NeoForge" (Sinytra Connector): a NeoForge search also takes Fabric-only mods, which the
      // Install button then offers "via Connector". Without this they never came back from the search at all.
      const asked = loaderApplies ? loaders : [];
      const withFabric = settings.connector && app.browseType === "mod" && asked.includes("neoforge") && !asked.includes("fabric") ? [...asked, "fabric"] : asked;
      const [mr, cf] = await Promise.all([
        wantMr ? api.search({
          query: app.browseQuery, projectType: app.browseType,
          loaders: withFabric,
          gameVersions: gameVersion ? [gameVersion] : [],
          categories: [...categories], sort, offset: mrOffset, limit: 40,
        }) : null,
        wantCf ? api.cfSearch({
          query: app.browseQuery, projectType: app.browseType, loader: "", loaders: withFabric,
          gameVersion, sort, offset: cfOffset, limit: 40,
        }).catch(() => null) : null,
      ]);
      if (gen !== generation) return;
      if (mr) { mrOffset += mr.hits.length; mrTotal = mr.total_hits; }
      if (cf) { cfOffset += cf.data.length; cfTotal = Math.min(cf.pagination.totalCount, 10000); }
      const page = mergeHits(mr?.hits ?? [], (cf?.data ?? []).map(cfToHit));
      // Drop anything already shown (the same project can arrive on a later page from the other site).
      const seen = new Set(reset ? [] : hits.flatMap((h) => [h.project_id, h.cf_id ? `cf:${h.cf_id}` : ""]));
      const fresh = page.filter((h) => !seen.has(h.project_id) && !(h.cf_id && seen.has(`cf:${h.cf_id}`)));
      if (sort === "downloads") fresh.sort((a, b) => b.downloads - a.downloads);
      hits = reset ? fresh : [...hits, ...fresh];
      total = mrTotal + (useCf ? cfTotal : 0);
    } catch (e) {
      if (gen === generation) error = String(e);
    } finally {
      if (gen === generation) { loading = false; topUp(); }
    }
  }

  function setType(t: ProjectType) {
    app.browseType = t;
    categories = new Set();
  }

  function toggleCategory(name: string) {
    const s = new Set(categories);
    s.has(name) ? s.delete(name) : s.add(name);
    categories = s;
  }

  const shownCats = (h: Hit) => (h.display_categories ?? h.categories).filter((c) => !loaderNames.has(c)).slice(0, 4);
  const open = (h: Hit) => pushOverlay({ kind: "project", hit: h });
</script>

<svelte:window onclick={() => { showFilters = false; showSort = false; }} />

<div class="browse">
  <div class="bar">
    <select class="type" value={app.browseType} onchange={(e) => setType(e.currentTarget.value as ProjectType)} aria-label="Project type">
      {#each TYPES as t (t.type)}<option value={t.type}>{t.label}</option>{/each}
    </select>
    <span class="found">{total.toLocaleString()} projects found</span>
    <select class="type src" bind:value={source} aria-label="Source" title="Where to search">
      <option value="all">All sources</option>
      <option value="modrinth">Modrinth</option>
      <option value="curseforge" disabled={cfOk !== true}>CurseForge</option>
    </select>
    <div class="spacer"></div>

    <div class="pop">
      <button class="tool" onclick={(e) => { e.stopPropagation(); showSort = !showSort; showFilters = false; }}>
        ⇅ Sort: {SORTS.find((s) => s.id === sort)?.label}
      </button>
      {#if showSort}
        <div class="menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
          {#each SORTS as s (s.id)}
            <button class:on={sort === s.id} onclick={() => { sort = s.id; showSort = false; }}>{s.label}{#if sort === s.id}<span>✓</span>{/if}</button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="pop">
      <button class="tool" class:active={categories.size || hideInstalled || hideLibraries} onclick={(e) => { e.stopPropagation(); showFilters = !showFilters; showSort = false; }}>
        Filters{categories.size ? ` (${categories.size})` : ""}
      </button>
      {#if showFilters}
        <div class="menu filters" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
          <label class="check"><input type="checkbox" bind:checked={hideInstalled} /> Hide installed content</label>
          <label class="check"><input type="checkbox" bind:checked={hideLibraries} /> Hide libraries and APIs</label>
          {#if loaderApplies}
            <div class="field">Modloader <small class="muted">(none ticked = any)</small>
              {#each ["neoforge", "forge", "fabric", "quilt"] as l (l)}
                <label class="check"><input type="checkbox" checked={loaders.includes(l)}
                  onchange={() => (loaders = loaders.includes(l) ? loaders.filter((x) => x !== l) : [...loaders, l])} /> {cap(l)}</label>
              {/each}
            </div>
          {/if}
          <label class="field">Game version
            <select bind:value={gameVersion}>
              <option value="">Any</option>
              {#each tags?.gameVersions ?? [] as v (v)}<option value={v}>{v}</option>{/each}
            </select>
          </label>
          {#if typeCategories.length}
            <div class="cats">
              <strong>Categories</strong>
              {#each typeCategories as c (c.name)}
                <label class="check"><input type="checkbox" checked={categories.has(c.name)} onchange={() => toggleCategory(c.name)} /> {cap(c.name)}</label>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>

    {#if view === "tiles"}<input class="slider" type="range" min="140" max="260" step="10" bind:value={tile} aria-label="Tile size" />{/if}
    <div class="views" role="group" aria-label="View">
      {#each [["list", "M4 6h16M4 12h16M4 18h16"], ["table", "M4 5h16v14H4zM4 10h16M4 15h16M10 5v14"], ["tiles", "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z"]] as [v, d] (v)}
        <button class:on={view === v} title={cap(v)} aria-label="{cap(v)} view" aria-pressed={view === v} onclick={() => (view = v as any)}>
          <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path {d} fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" /></svg>
        </button>
      {/each}
    </div>
  </div>

  <div class="target">
    <TargetChip />
    {#if app.browseQuery}<span class="q">Results for “{app.browseQuery}” <button class="link" onclick={() => (app.browseQuery = "")}>clear</button></span>{/if}
  </div>
  {#if error}<div class="error">{error}</div>{/if}
  {#if cfOk === false && source !== "modrinth"}
    <div class="note">CurseForge results are off: {#if hasKey}your API key can't use CurseForge search (other CurseForge features still work){:else}add a CurseForge API key in Settings to search CurseForge too{/if}. <button class="link" onclick={() => { app.page = "settings"; app.settingsSection = "curseforge"; }}>Settings</button></div>
  {:else if useCf && categories.size}
    <div class="note">Category filters use Modrinth's categories, so CurseForge results are hidden while one is picked.</div>
  {/if}

  <div class="results">
    {#if view === "tiles"}
      <div class="tiles" style:--tile="{tile}px">
        {#each shown as h (h.project_id)}
          <button class="tilecard" onclick={() => open(h)}>
            <div class="art">{#if h.icon_url}<img src={h.icon_url} alt="" loading="lazy" decoding="async" />{/if}</div>
            <span class="tile-src"><SourceBadges hit={h} /></span>
            <strong>{h.title}</strong><small>By {h.author}</small>
          </button>
        {/each}
      </div>
    {:else}
      {#each shown as h (h.project_id)}
        <div class="card" class:compact={view === "table"}>
          <button class="icon" onclick={() => open(h)} aria-label="Open {h.title}">
            {#if h.icon_url}<img src={h.icon_url} alt="" loading="lazy" decoding="async" />{/if}
          </button>
          <div class="body">
            <div class="title-row">
              <button class="title" onclick={() => open(h)}>{h.title}</button>
              <span class="by">by {h.author}</span>
              <SourceBadges hit={h} />
            </div>
            {#if view === "list"}<p class="desc">{h.description}</p>{/if}
            <div class="meta">
              {#if view === "list"}
                <span class="chip type-chip">{typeLabel}</span>
                {#each shownCats(h) as c (c)}<span class="chip">{cap(c)}</span>{/each}
              {/if}
              <span class="stats">
                <span title="Downloads">⤓ {compact(h.downloads)}</span>
                <span title="Last updated">◷ {ago(h.date_modified)}</span>
                <span title="Game versions">{versionsLabel(h.versions)}</span>
                {#if envLabel(h.client_side, h.server_side) && app.browseType === "mod"}<span>{envLabel(h.client_side, h.server_side)}</span>{/if}
                {#if loaderOf(h).length}<span>{cap(loaderOf(h)[0])}{loaderOf(h).length > 1 ? ` + ${loaderOf(h).length - 1}` : ""}</span>{/if}
              </span>
            </div>
          </div>
          <div class="act"><InstallButton hit={h} /></div>
        </div>
      {/each}
    {/if}
    {#if !loading && shown.length === 0}<div class="empty">Nothing found. Try fewer filters.</div>{/if}
    <div bind:this={sentinel} class="sentinel">{#if loading}Loading…{/if}</div>
  </div>
</div>

<style>
  .browse { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .bar { display: flex; align-items: center; gap: 10px; padding: 12px 0 8px; }
  .type { background: none; border: 0; color: var(--text); font: inherit; font-size: 15px; font-weight: 600; padding: 6px 4px; cursor: pointer; }
  .type option, .field select option { background: var(--panel); }
  .found { color: var(--muted); margin-left: 14px; }
  .spacer { flex: 1; }
  .tool { background: none; border: 0; padding: 7px 10px; border-radius: 6px; color: var(--text); }
  .tool:hover, .tool.active { background: var(--panel); }
  .pop { position: relative; }
  .menu { position: absolute; right: 0; top: calc(100% + 6px); z-index: 20; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 6px; min-width: 190px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.45); }
  .menu > button { display: flex; justify-content: space-between; width: 100%; background: none; border: 0; padding: 7px 10px; border-radius: 5px; text-align: left; }
  .menu > button:hover, .menu > button.on { background: var(--panel); }
  .filters { width: 290px; padding: 12px; display: flex; flex-direction: column; gap: 12px; max-height: 70vh; overflow-y: auto; }
  .check { display: flex; align-items: center; gap: 8px; cursor: pointer; }
  .field { display: flex; flex-direction: column; gap: 5px; font-weight: 600; }
  .field select { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; font: inherit; font-weight: 400; }
  .cats { display: flex; flex-direction: column; gap: 6px; border-top: 1px solid var(--line); padding-top: 10px; }
  .slider { width: 100px; accent-color: var(--accent); }
  .views { display: flex; gap: 2px; background: var(--panel); border-radius: 6px; padding: 2px; }
  .views button { display: grid; place-items: center; background: none; border: 0; padding: 5px 9px; border-radius: 5px; color: var(--muted); }
  /* Block themes frame every button: space them so the frames don't run into each other. */
  :global(:root[data-skin="block"]) .views { gap: 6px; background: none; padding: 0; }
  .views button.on { background: var(--panel-2); color: var(--accent); }
  .target { display: flex; align-items: center; gap: 14px; color: var(--muted); font-size: 13px; padding-bottom: 8px; }
  .link { background: none; border: 0; color: var(--accent); padding: 0 2px; }
  .error { background: var(--err-bg); color: var(--err-text); padding: 8px 12px; border-radius: 6px; margin-bottom: 8px; }

  .results { flex: 1; overflow-y: auto; min-height: 0; padding-bottom: 20px; display: flex; flex-direction: column; gap: 8px; }
  .card { display: grid; grid-template-columns: 108px 1fr auto; gap: 16px; background: var(--panel); border-radius: 8px; padding: 10px 14px 10px 10px; content-visibility: auto; contain-intrinsic-size: auto 128px; }
  .card.compact { grid-template-columns: 44px 1fr auto; padding: 8px 14px 8px 8px; align-items: center; contain-intrinsic-size: auto 60px; }
  .icon { width: 108px; height: 108px; padding: 0; border: 0; border-radius: 6px; overflow: hidden; background: var(--panel-2); }
  .compact .icon { width: 44px; height: 44px; }
  .icon img, .art img { width: 100%; height: 100%; object-fit: cover; }
  .body { min-width: 0; display: flex; flex-direction: column; gap: 6px; justify-content: center; }
  .title-row { display: flex; align-items: baseline; gap: 10px; min-width: 0; }
  .title { background: none; border: 0; padding: 0; font-size: 17px; font-weight: 600; text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .title:hover { color: var(--accent); }
  .by { color: var(--muted); font-size: 13px; white-space: nowrap; }
  .desc { margin: 0; color: var(--text-2); font-size: 13.5px; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .meta { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; border-top: 1px solid var(--line); padding-top: 7px; }
  .compact .meta { border: 0; padding: 0; }
  .chip { font-size: 12px; color: var(--text-2); padding: 1px 6px; }
  .type-chip { border: 1px solid var(--line); border-radius: 4px; }
  .stats { margin-left: auto; display: flex; gap: 16px; color: var(--muted); font-size: 12.5px; white-space: nowrap; }
  .act { display: flex; align-items: flex-start; }
  .compact .act { align-items: center; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, var(--tile)); gap: 12px; }
  .note { background: var(--panel); color: var(--muted); border-radius: 6px; padding: 8px 12px; margin-bottom: 10px; font-size: 13px; }
  .tile-src { position: absolute; top: 6px; right: 6px; }
  .tilecard { position: relative; display: flex; flex-direction: column; padding: 0 0 10px; border: 0; background: var(--panel); border-radius: 8px; overflow: hidden; text-align: left; transition: transform 0.15s; }
  .tilecard:hover { transform: translateY(-2px); }
  .art { aspect-ratio: 1; background: var(--panel-2); margin-bottom: 8px; }
  .tilecard strong, .tilecard small { padding: 0 10px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tilecard small { color: var(--muted); }
  .sentinel { min-height: 40px; display: grid; place-items: center; color: var(--muted); }
  .empty { color: var(--muted); text-align: center; padding: 40px; }
</style>
