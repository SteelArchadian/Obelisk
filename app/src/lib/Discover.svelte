<script lang="ts">
  import { api, compact, TYPES, type Hit, type ProjectType } from "./api";
  import { app, openBrowse, pushOverlay } from "./state.svelte";

  type Row = { title: string; type: ProjectType; hits: Hit[] };

  let featured = $state<Hit[]>([]);
  let slide = $state(0);
  let rows = $state<Row[]>([]);
  let error = $state("");

  const mc = app.target?.minecraft ?? app.current?.minecraft ?? "";
  const loader = (app.target ?? app.current)?.loader.toLowerCase() ?? "";

  const base = { query: "", loaders: [] as string[], gameVersions: [] as string[], categories: [] as string[], offset: 0 };

  $effect(() => {
    api.search({ ...base, projectType: "modpack", sort: "follows", limit: 10 })
      .then((r) => (featured = r.hits.filter((h) => h.featured_gallery || h.gallery?.length)))
      .catch((e) => (error = String(e)));
    const specs: [string, ProjectType, string, boolean][] = [
      [`Popular mods${mc ? ` for ${mc}` : ""}`, "mod", "downloads", true],
      ["Recently updated mods", "mod", "updated", true],
      ["Trending shaders", "shader", "follows", false],
      ["Popular resource packs", "resourcepack", "downloads", false],
      ["Newest modpacks", "modpack", "newest", false],
    ];
    Promise.all(specs.map(([title, type, sort, matchInstance]) =>
      api.search({
        ...base, projectType: type, sort, limit: 12,
        loaders: matchInstance && loader ? [loader] : [],
        gameVersions: matchInstance && mc ? [mc] : [],
      }).then((r) => ({ title, type, hits: r.hits })),
    )).then((r) => (rows = r)).catch((e) => (error = String(e)));
  });

  $effect(() => {
    if (featured.length < 2) return;
    const t = setInterval(() => (slide = (slide + 1) % featured.length), 7000);
    return () => clearInterval(t);
  });

  let hero = $derived(featured[slide]);
</script>

<div class="discover">
  {#if error}<div class="error">{error}</div>{/if}

  {#if hero}
    <section class="hero" style:background-image="url({hero.featured_gallery || hero.gallery?.[0]})">
      <div class="shade"></div>
      <button class="nav prev" aria-label="Previous" onclick={() => (slide = (slide - 1 + featured.length) % featured.length)}>‹</button>
      <div class="hero-body">
        <div class="hero-title">{#if hero.icon_url}<img src={hero.icon_url} alt="" />{/if}<h2>{hero.title}</h2></div>
        <p>{hero.description}</p>
        <button class="cta" onclick={() => pushOverlay({ kind: "project", hit: hero })}>View modpack</button>
      </div>
      <button class="nav next" aria-label="Next" onclick={() => (slide = (slide + 1) % featured.length)}>›</button>
      <div class="dots">{#each featured as f, i (f.project_id)}<button class:on={i === slide} aria-label="Slide {i + 1}" onclick={() => (slide = i)}></button>{/each}</div>
    </section>
  {/if}

  {#each rows as row (row.title)}
    <section class="row">
      <div class="row-head"><h3>{row.title}</h3><button class="more" onclick={() => openBrowse(row.type)}>Browse all</button></div>
      <div class="strip">
        {#each row.hits as h (h.project_id)}
          <button class="card" onclick={() => pushOverlay({ kind: "project", hit: h })}>
            <div class="art">{#if h.icon_url}<img src={h.icon_url} alt="" loading="lazy" />{/if}</div>
            <strong>{h.title}</strong>
            <small>By {h.author}</small>
            <small>⤓ {compact(h.downloads)}</small>
          </button>
        {/each}
      </div>
    </section>
  {/each}

  <section class="row">
    <div class="row-head"><h3>Popular categories</h3></div>
    <div class="cats">
      {#each TYPES as t (t.type)}<button onclick={() => openBrowse(t.type)}>{t.label}</button>{/each}
    </div>
  </section>
</div>

<style>
  .discover { flex: 1; overflow-y: auto; min-height: 0; padding: 16px 0 40px; }
  .error { background: var(--err-bg); color: var(--err-text); padding: 8px 12px; border-radius: 6px; margin-bottom: 10px; }
  .hero { position: relative; height: 380px; border-radius: 10px; overflow: hidden; background-size: cover; background-position: center; display: flex; align-items: flex-end; transition: background-image 0.4s; }
  .shade { position: absolute; inset: 0; background: linear-gradient(180deg, transparent 35%, rgb(0 0 0 / 0.85)); }
  .hero-body { position: relative; padding: 24px 70px 34px; max-width: 760px; }
  .hero-title { display: flex; align-items: center; gap: 12px; }
  .hero-title img { width: 44px; height: 44px; border-radius: 8px; }
  .hero h2 { margin: 0; font-size: 24px; }
  .hero p { color: var(--text-2); margin: 8px 0 14px; }
  .cta { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 9px 18px; font-weight: 600; }
  .cta:hover { background: var(--accent-hover); }
  .nav { position: absolute; top: 50%; transform: translateY(-50%); width: 36px; height: 36px; border-radius: 50%; border: 0; background: rgb(0 0 0 / 0.5); color: #fff; font-size: 22px; line-height: 1; z-index: 1; }
  .prev { left: 14px; }
  .next { right: 14px; }
  .dots { position: absolute; bottom: 12px; left: 50%; transform: translateX(-50%); display: flex; gap: 8px; }
  .dots button { width: 8px; height: 8px; border-radius: 50%; border: 0; padding: 0; background: rgb(255 255 255 / 0.4); }
  .dots button.on { background: #fff; }
  .row { margin-top: 26px; }
  .row-head { display: flex; align-items: center; gap: 14px; margin-bottom: 10px; }
  .row-head h3 { margin: 0; font-size: 16px; font-weight: 600; }
  .more { background: var(--panel-2); border: 0; border-radius: 4px; padding: 3px 10px; font-size: 13px; }
  .strip { display: grid; grid-auto-flow: column; grid-auto-columns: 172px; gap: 12px; overflow-x: auto; padding-bottom: 8px; }
  .card { display: flex; flex-direction: column; text-align: left; padding: 0 0 10px; border: 0; background: var(--panel); border-radius: 8px; overflow: hidden; transition: transform 0.15s; }
  .card:hover { transform: translateY(-2px); }
  .art { aspect-ratio: 1; background: var(--panel-2); margin-bottom: 8px; }
  .art img { width: 100%; height: 100%; object-fit: cover; }
  .card strong, .card small { padding: 0 10px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .card small { color: var(--muted); font-size: 12.5px; }
  .cats { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 12px; }
  .cats button { background: var(--panel); border: 0; border-radius: 6px; padding: 20px; font-size: 16px; color: var(--text-2); transition: background 0.15s; }
  .cats button:hover { background: var(--panel-2); color: var(--text); }
</style>
