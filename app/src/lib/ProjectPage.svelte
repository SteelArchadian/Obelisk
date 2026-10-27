<script lang="ts">
  import { crisp } from "./crisp";
  import { api, compact, ago, versionsLabel, type Hit, type Version } from "./api";
  import { app, popOverlay, closeOverlays, openBrowse, pushOverlay } from "./state.svelte";
  import { cap, install, isInstalled, linkTarget, openProjectById } from "./install";
  import { cfFileToVersion } from "./cf";
  import SourceBadges from "./SourceBadges.svelte";
  import { renderBody, wireYouTube } from "./richtext";
  import InstallButton from "./InstallButton.svelte";
  import TargetChip from "./TargetChip.svelte";
  import Relations from "./Relations.svelte";
  import { mr, toggleFollow, openLink } from "./account.svelte";

  let { hit }: { hit: Hit } = $props();

  let tab = $state<"overview" | "changelog" | "gallery" | "versions" | "relations">("overview");
  let project = $state<any>(null);
  let fetchedAuthor = $state("");
  let author = $derived(hit.author || fetchedAuthor);
  let versions = $state<Version[]>([]);
  let showAll = $state(false);
  let busyVersion = $state("");
  let changelogAt = $state<number | null>(null);
  let lightbox = $state<string | null>(null);
  let bodyEl = $state<HTMLElement>();
  let error = $state("");

  let body = $derived(project ? renderBody(project.body ?? "") : "");
  // Modrinth's `url` is a small still preview; `raw_url` is the original (full size, and GIFs animate).
  let gallery = $derived<{ url: string; full: string; title?: string }[]>(
    (project?.gallery?.length ? project.gallery : (hit.gallery ?? []).map((url) => ({ url }))).map((g: any) => {
      const full = g.raw_url ?? g.url;
      return { url: /\.gif($|\?)/i.test(full) ? full : g.url, full, title: g.title };
    }),
  );
  let installedVersion = $derived(app.target ? isInstalled(hit)?.versionId ?? "" : "");
  /** A CurseForge-only project (Modrinth projects that are also on CurseForge use Modrinth data). */
  let isCf = $derived(hit.project_id.startsWith("cf:"));

  /** Versions that fit the target profile (loader + game version), unless "show incompatible" is on. */
  let fits = (v: Version) => {
    const t = app.target;
    if (!t) return true;
    const loaderOk = hit.project_type !== "mod" && hit.project_type !== "modpack" ? true : v.loaders.includes(t.loader.toLowerCase());
    return loaderOk && v.game_versions.includes(t.minecraft);
  };
  let listed = $derived(showAll ? versions : versions.filter(fits));

  $effect(() => {
    const id = hit.project_id;
    if (id.startsWith("cf:")) {
      const cfId = hit.cf_id!;
      api.cfProject(cfId).then(({ project: p, description }) => {
        project = {
          body: description,
          gallery: (p.screenshots ?? []).map((g: any) => ({ url: g.url, title: g.title })),
          loaders: [...new Set((p.latestFilesIndexes ?? []).map((i: any) => ({ 1: "forge", 4: "fabric", 5: "quilt", 6: "neoforge" } as Record<number, string>)[i.modLoader]).filter(Boolean))],
          source_url: p.links?.sourceUrl, issues_url: p.links?.issuesUrl, wiki_url: p.links?.wikiUrl,
          site_url: p.links?.websiteUrl,
        };
      }).catch((e) => (error = String(e)));
      api.cfFiles(cfId).then((f) => (versions = f.map((x) => cfFileToVersion(x, cfId)))).catch(() => {});
      return;
    }
    api.project(id).then((p) => (project = p)).catch((e) => (error = String(e)));
    api.versions(id, true).then((v) => (versions = v)).catch(() => {});
    if (!hit.author) {
      api.members(id).then((m) => {
        const owner = m.find((x) => x.role === "Owner") ?? m[0];
        if (owner) fetchedAuthor = owner.user.username;
      }).catch(() => {});
    }
  });

  $effect(() => {
    if (bodyEl && body && tab === "overview") queueMicrotask(() => bodyEl && wireYouTube(bodyEl));
  });

  // Links to Modrinth or CurseForge mod, author and search pages open inside Obelisk; anything else in the browser.
  async function onClick(e: MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (!a?.href || !/^https?:/.test(a.href)) return;
    e.preventDefault();
    const t = await linkTarget(a.href);
    if (t && "project" in t) openProjectById(t.project);
    else if (t && "search" in t) openBrowse(t.search, t.query);
    else if (t && "author" in t && !t.cf) pushOverlay({ kind: "author", name: t.author });
    // A CurseForge member is found by id; we know it only for this project's own author.
    else if (t && "author" in t && isCf && t.author.toLowerCase() === author.toLowerCase()) openAuthor();
    else openLink(a.href);
  }

  async function installVersion(v: Version) {
    busyVersion = v.id;
    const cross = !isCf && app.target && hit.project_type === "mod" && !v.loaders.includes(app.target.loader.toLowerCase());
    if (cross && v.loaders.includes("fabric") && app.target?.loader.toLowerCase() === "neoforge") {
      app.connector = { hit, versionId: v.id, loaderOverride: "fabric" };
    } else {
      await install({ hit, versionId: v.id });
    }
    busyVersion = "";
  }

  const changelogVersion = $derived(changelogAt === null ? null : listed[changelogAt]);

  // CurseForge changelogs are one request per file, so they load only when shown.
  const loadingCl = new Set<string>();
  function loadChangelogs(list: Version[]) {
    for (const v of list) {
      if (v.changelog !== null || loadingCl.has(v.id)) continue;
      loadingCl.add(v.id);
      api.cfChangelog(hit.cf_id!, Number(v.id))
        .then((c) => (versions = versions.map((x) => (x.id === v.id ? { ...x, changelog: c || "<p><em>No changelog.</em></p>" } : x))))
        .catch(() => loadingCl.delete(v.id));
    }
  }
  $effect(() => {
    if (!isCf) return;
    if (tab === "changelog") loadChangelogs(listed.slice(0, 10));
    if (changelogVersion) loadChangelogs([changelogVersion]);
  });

  async function openAuthor() {
    if (!isCf) return pushOverlay({ kind: "author", name: author });
    const a = (await api.cfProject(hit.cf_id!).catch(() => null))?.project?.authors?.[0];
    if (a?.id) pushOverlay({ kind: "author", name: a.name, cfId: a.id });
    else openLink(`https://www.curseforge.com/members/${encodeURIComponent(author)}/projects`);
  }

  // Gallery slideshow: A/W/←/↑ back, D/S/→/↓ forward (opens the first image when none is open).
  let shown = $derived(lightbox ? gallery.findIndex((g) => g.full === lightbox) : -1);
  function step(by: number) {
    if (!gallery.length) return;
    const i = shown < 0 ? (by > 0 ? 0 : gallery.length - 1) : (shown + by + gallery.length) % gallery.length;
    lightbox = gallery[i].full;
  }

  // Lightbox view: the image fits the screen (small ones are enlarged); scroll over it or use the buttons to
  // zoom towards the cursor, drag to move around while zoomed.
  let fit = $state({ w: 0, h: 0 });
  let zoom = $state(1);
  let pan = $state({ x: 0, y: 0 });
  let drag: { x: number; y: number; px: number; py: number; moved: boolean } | null = null;
  $effect(() => { void lightbox; zoom = 1; pan = { x: 0, y: 0 }; fit = { w: 0, h: 0 }; });
  function fitImage(e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    const k = Math.min((innerWidth * 0.9) / img.naturalWidth, (innerHeight * 0.82) / img.naturalHeight);
    fit = { w: Math.round(img.naturalWidth * k), h: Math.round(img.naturalHeight * k) };
  }
  function zoomTo(z: number, cx = fit.w / 2, cy = fit.h / 2) {
    const nz = Math.min(8, Math.max(1, z));
    // Keep the point under the cursor in place.
    pan = nz === 1 ? { x: 0, y: 0 } : { x: cx - ((cx - pan.x) * nz) / zoom, y: cy - ((cy - pan.y) * nz) / zoom };
    zoom = nz;
  }
  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    zoomTo(zoom * (e.deltaY < 0 ? 1.2 : 1 / 1.2), e.clientX - r.left, e.clientY - r.top);
  }
  function slide(e: KeyboardEvent) {
    if (tab !== "gallery" || e.ctrlKey || e.altKey || e.metaKey) return false;
    if ((e.target as HTMLElement)?.closest?.("input, textarea, select, [contenteditable]")) return false;
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const by = ["a", "w", "ArrowLeft", "ArrowUp"].includes(k) ? -1 : ["d", "s", "ArrowRight", "ArrowDown"].includes(k) ? 1 : 0;
    if (!by) return false;
    e.preventDefault();
    step(by);
    return true;
  }
</script>

<svelte:window onkeydown={(e) => {
  if (slide(e)) return;
  if (e.key !== "Escape") return;
  if (lightbox) lightbox = null;
  else if (changelogAt !== null) changelogAt = null;
  else popOverlay();
}} />

<div class="page" role="dialog" aria-label={hit.title}>
  <div class="crumbs">
    {#if app.stack.length > 1}<button class="nav" onclick={popOverlay}>‹ Back</button>{/if}
    <TargetChip {hit} />
    <button class="x" onclick={closeOverlays} aria-label="Close">✕</button>
  </div>

  <header class="head">
    <div class="icon">{#if hit.icon_url}<img src={hit.icon_url} alt="" />{/if}</div>
    <div class="info">
      <div class="row1">
        <h2>{hit.title}</h2>
        {#if author}<button class="by" onclick={openAuthor}>by {author}</button>{/if}
        <SourceBadges {hit} />
        <div class="spacer"></div>
        {#if mr.account && !isCf}
          <button class="follow" class:on={mr.follows.has(hit.project_id)} title={mr.follows.has(hit.project_id) ? "Following on Modrinth (click to unfollow)" : "Follow on Modrinth"}
            aria-pressed={mr.follows.has(hit.project_id)} onclick={() => toggleFollow(hit.project_id)}>{mr.follows.has(hit.project_id) ? "♥" : "♡"}</button>
        {/if}
        <InstallButton {hit} big />
      </div>
      <p class="desc">{hit.description}</p>
      <div class="row3">
        <span class="chip">{cap(hit.project_type)}</span>
        {#each (hit.display_categories ?? hit.categories).slice(0, 4) as c (c)}<span class="chip plain">{cap(c)}</span>{/each}
        <span class="stats">
          <span title="Downloads">⤓ {compact(hit.downloads)}</span>
          <span title="Last updated">◷ {ago(hit.date_modified)}</span>
          <span title="Game versions">{versionsLabel(hit.versions)}</span>
          {#if project?.loaders?.length}<span>{project.loaders.map(cap).slice(0, 2).join(", ")}{project.loaders.length > 2 ? ` + ${project.loaders.length - 2}` : ""}</span>{/if}
        </span>
      </div>
    </div>
  </header>

  <nav class="tabs">
    {#each [["overview", "Description"], ["changelog", "Changelog"], ["versions", `${isCf ? "Files" : "Versions"} (${listed.length})`], ["gallery", `Gallery (${gallery.length})`], ["relations", "Relations"]] as [id, label] (id)}
      <button class:on={tab === id} onclick={() => (tab = id as any)}>{label}</button>
    {/each}
    {#if project}
      <div class="links">
        {#if isCf && project.site_url}<button class="link" title="CurseForge's comments (in your browser)" onclick={() => openLink(`${project.site_url.replace(/\/$/, "")}/comments`)}>Comments ↗</button>{/if}
        {#each [["issues_url", "Issues"], ["source_url", "Source"], ["wiki_url", "Wiki"], ["discord_url", "Discord"]] as [k, label] (k)}
          {#if project[k]}<button class="link" title={project[k]} onclick={() => openLink(project[k])}>{label} ↗</button>{/if}
        {/each}
        {#if !isCf}<button class="link" onclick={() => openLink(`https://modrinth.com/${hit.project_type}/${hit.slug}`)}>Modrinth ↗</button>{/if}
        {#if isCf || hit.cf_id}<button class="link" onclick={() => openLink(project?.site_url ?? `https://www.curseforge.com/projects/${hit.cf_id}`)}>CurseForge ↗</button>{/if}
      </div>
    {/if}
  </nav>

  {#if error}<div class="error">{error}</div>{/if}

  <div class="content" onclick={onClick} role="presentation">
    {#if tab === "overview"}
      {#if project}<article class="rich" bind:this={bodyEl}>{@html body}</article>{:else}<div class="muted">Loading…</div>{/if}
    {:else if tab === "changelog"}
      {#each listed.slice(0, 20) as v (v.id)}
        <section class="change">
          <h3>{v.name} <small>{v.version_type} · {ago(v.date_published)}</small></h3>
          <div class="rich">{@html v.changelog === null && isCf ? '<p class="muted">Loading…</p>' : renderBody(v.changelog || "_No changelog._")}</div>
        </section>
      {:else}<div class="muted">{versions.length ? "No versions for this profile. Turn on “Show incompatible versions” in Versions." : "Loading…"}</div>{/each}
    {:else if tab === "relations"}
      <Relations {hit} cfId={hit.cf_id ?? null} deps={(versions[0]?.dependencies ?? []).filter((d) => d.project_id) as { project_id: string; dependency_type: string }[]} />
    {:else if tab === "gallery"}
      <div class="gallery">
        {#each gallery as g (g.url)}
          <button class="shot" onclick={() => (lightbox = g.full)}>
            <img src={g.url} alt={g.title ?? ""} loading="lazy" />
            {#if g.title}<span>{g.title}</span>{/if}
          </button>
        {:else}<div class="muted">No gallery images.</div>{/each}
      </div>
    {:else}
      <div class="vbar">
        {#if app.target}<span class="muted small">Showing versions for {app.target.loader} {app.target.minecraft}</span>{/if}
        <label class="toggle">Show incompatible versions
          <button class="switch" class:on={showAll} role="switch" aria-checked={showAll} aria-label="Show incompatible versions" onclick={() => (showAll = !showAll)}><i></i></button>
        </label>
      </div>
      <div class="vtable">
        <div class="vrow vhead"><span>Type</span><span>Name</span><span>Released</span><span>Loader</span><span>Game version</span><span>Action</span><span></span></div>
        {#each listed as v, i (v.id)}
          <div class="vrow" class:dim={!fits(v)}>
            <span><span class="vt {v.version_type}">{v.version_type[0].toUpperCase()}</span></span>
            <span class="ellipsis" title={v.version_number}>{v.name}</span>
            <span>{new Date(v.date_published).toLocaleDateString()}</span>
            <span class="ellipsis">{v.loaders.map(cap).join(", ")}</span>
            <span class="ellipsis">{versionsLabel(v.game_versions)}</span>
            <span>
              {#if installedVersion === v.id}
                <span class="installed">Installed</span>
              {:else if hit.project_type === "datapack"}
                <span class="muted small">World only</span>
              {:else}
                <button class="vinstall" disabled={busyVersion === v.id} onclick={() => installVersion(v)}>
                  {busyVersion === v.id ? "…" : installedVersion ? "Switch" : "Install"}
                </button>
              {/if}
            </span>
            <span><button class="cl" title="Changelog" aria-label="Changelog for {v.name}" onclick={() => (changelogAt = i)}>
              <svg viewBox="0 0 24 24" width="18" height="18"><path d="M6 3h9l4 4v14H6z M14 3v5h5 M9 12h7 M9 16h7" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" /></svg>
            </button></span>
          </div>
        {:else}
          <div class="muted pad">{versions.length ? "No versions match this profile." : "Loading…"}</div>
        {/each}
      </div>
    {/if}
  </div>
</div>

{#if changelogVersion}
  <div class="modal-back" role="presentation" onclick={() => (changelogAt = null)}>
    <div class="modal" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      <button class="mx" onclick={() => (changelogAt = null)} aria-label="Close">✕</button>
      <h3>{changelogVersion.files.find((f) => f.primary)?.filename ?? changelogVersion.name}</h3>
      <p class="muted small">{new Date(changelogVersion.date_published).toLocaleDateString(undefined, { dateStyle: "medium" })}</p>
      <div class="rich modal-body">{@html changelogVersion.changelog === null && isCf ? "Loading…" : renderBody(changelogVersion.changelog || "_No changelog._")}</div>
      <div class="modal-nav">
        <button disabled={changelogAt === listed.length - 1} onclick={() => changelogAt!++}>‹ Previous</button>
        <button disabled={changelogAt === 0} onclick={() => changelogAt!--}>Next ›</button>
      </div>
    </div>
  </div>
{/if}

{#if lightbox}
  <div class="modal-back" role="presentation" onclick={() => (lightbox = null)}>
    <figure class="lbwrap">
      <div class="lbview" role="presentation" style:width="{fit.w || undefined}px" style:height="{fit.h || undefined}px" class:zoomed={zoom > 1}
        onwheel={onWheel}
        onpointerdown={(e) => { drag = { x: e.clientX, y: e.clientY, px: pan.x, py: pan.y, moved: false }; }}
        onpointermove={(e) => { if (!drag || zoom === 1) return; drag.moved = true; pan = { x: drag.px + e.clientX - drag.x, y: drag.py + e.clientY - drag.y }; }}
        onpointerup={() => setTimeout(() => (drag = null))}
        onclick={(e) => { if (zoom > 1 || drag?.moved) e.stopPropagation(); }}>
        <img class="lb" use:crisp src={lightbox} alt="" onload={fitImage} draggable="false"
          style:width="{fit.w || undefined}px" style:height="{fit.h || undefined}px"
          style:transform="translate({pan.x}px, {pan.y}px) scale({zoom})" />
      </div>
      <div class="lbzoom" role="toolbar" aria-label="Zoom">
        <button aria-label="Zoom out" onclick={(e) => { e.stopPropagation(); zoomTo(zoom / 1.5); }}>−</button>
        <button onclick={(e) => { e.stopPropagation(); zoomTo(1); }}>{Math.round(zoom * 100)}%</button>
        <button aria-label="Zoom in" onclick={(e) => { e.stopPropagation(); zoomTo(zoom * 1.5); }}>+</button>
      </div>
      <figcaption>{gallery[shown]?.title ?? ""}{gallery.length > 1 ? ` · ${shown + 1} / ${gallery.length} · A/D or arrow keys` : ""}</figcaption>
    </figure>
    {#if gallery.length > 1}
      <button class="lbnav prev" aria-label="Previous image" onclick={(e) => { e.stopPropagation(); step(-1); }}>‹</button>
      <button class="lbnav next" aria-label="Next image" onclick={(e) => { e.stopPropagation(); step(1); }}>›</button>
    {/if}
  </div>
{/if}

<style>
  .page { position: absolute; inset: 0; background: var(--bg); z-index: 15; display: flex; flex-direction: column; padding: 0 28px; overflow: hidden; animation: fade 0.12s ease-out; }
  @keyframes fade { from { opacity: 0; transform: translateY(6px); } }
  .crumbs { display: flex; align-items: center; gap: 12px; padding: 12px 0 10px; color: var(--muted); }
  .nav { background: none; border: 0; color: var(--muted); padding: 4px 8px; border-radius: 6px; }
  .nav:hover, .x:hover { color: var(--text); background: var(--panel); }
  .x { margin-left: auto; background: none; border: 0; font-size: 18px; color: var(--muted); padding: 4px 8px; border-radius: 6px; }
  .head { display: flex; gap: 16px; background: var(--panel); border-radius: 8px; padding: 10px 14px 10px 10px; }
  .icon { width: 108px; height: 108px; flex: none; border-radius: 6px; overflow: hidden; background: var(--panel-2); }
  .icon img { width: 100%; height: 100%; object-fit: cover; }
  .info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; justify-content: center; }
  .row1 { display: flex; align-items: center; gap: 12px; }
  h2 { margin: 0; font-size: 19px; font-weight: 600; }
  .by { background: none; border: 0; border-left: 1px solid var(--line); padding: 0 0 0 12px; color: var(--muted); }
  .by:hover { color: var(--accent); text-decoration: underline; }
  .spacer { flex: 1; }
  .desc { margin: 0; color: var(--text-2); font-size: 13.5px; }
  .row3 { display: flex; align-items: center; gap: 8px; border-top: 1px solid var(--line); padding-top: 7px; }
  .chip { font-size: 12px; color: var(--text-2); border: 1px solid var(--line); border-radius: 4px; padding: 1px 6px; }
  .chip.plain { border: 0; }
  .stats { margin-left: auto; display: flex; gap: 16px; color: var(--muted); font-size: 12.5px; white-space: nowrap; }
  .tabs { display: flex; flex-wrap: wrap; gap: 4px 28px; border-bottom: 1px solid var(--line); margin-top: 14px; align-items: center; }
  .tabs > button { background: none; border: 0; padding: 8px 0 10px; color: var(--muted); border-bottom: 2px solid transparent; margin-bottom: -1px; font-size: 15px; }
  .follow { width: 40px; height: 40px; flex: none; border-radius: 8px; border: 1px solid var(--line); background: var(--panel); color: var(--muted); font-size: 20px; line-height: 1; }
  .follow:hover { color: var(--text); }
  .follow.on { color: #ff5c7a; border-color: color-mix(in srgb, #ff5c7a 50%, var(--line)); }
  .tabs > button.on { color: var(--text); border-bottom-color: var(--accent); }
  .links { margin-left: auto; display: flex; gap: 12px; }
  .link { background: none; border: 0; color: var(--muted); font-size: 13px; padding: 2px; }
  .link:hover { color: var(--accent); }
  .error { background: var(--err-bg); color: var(--err-text); padding: 8px 12px; border-radius: 6px; margin-top: 10px; }
  .content { flex: 1; overflow-y: auto; min-height: 0; padding: 18px 0 40px; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  .pad { padding: 20px; }

  .rich { max-width: 980px; margin: 0 auto; line-height: 1.6; color: var(--text-2); user-select: text; overflow-wrap: anywhere; }
  .rich :global(img) { max-width: 100%; height: auto; border-radius: 4px; }
  .rich :global(iframe) { width: min(100%, 720px); aspect-ratio: 16 / 9; height: auto; border: 0; border-radius: 6px; display: block; margin: 12px 0; }
  .rich :global(a) { color: var(--accent); }
  .rich :global(h1), .rich :global(h2), .rich :global(h3) { color: var(--text); line-height: 1.25; }
  .rich :global(code) { background: var(--panel-2); padding: 1px 5px; border-radius: 4px; font-size: 0.9em; }
  .rich :global(pre) { background: var(--panel); padding: 12px; border-radius: 6px; overflow-x: auto; }
  .rich :global(blockquote) { border-left: 3px solid var(--line); margin: 0; padding-left: 14px; color: var(--muted); }
  .rich :global(details) { background: var(--panel); border-radius: 6px; padding: 8px 12px; margin: 8px 0; }
  .rich :global(summary) { cursor: pointer; color: var(--accent); }
  .rich :global(table) { border-collapse: collapse; }
  .rich :global(td), .rich :global(th) { border: 1px solid var(--line); padding: 4px 8px; }
  .rich :global(hr) { border: 0; border-top: 1px solid var(--line); }
  .change { max-width: 980px; margin: 0 auto 18px; background: var(--panel); border-radius: 8px; padding: 12px 16px; }
  .change h3 { margin: 0 0 6px; font-size: 15px; }
  .change small { color: var(--muted); font-weight: 400; margin-left: 6px; }
  .gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 12px; }
  .shot { position: relative; padding: 0; border: 0; border-radius: 8px; overflow: hidden; background: var(--panel); aspect-ratio: 16 / 9; }
  .shot img { width: 100%; height: 100%; object-fit: cover; transition: transform 0.2s; }
  .shot:hover img { transform: scale(1.03); }
  .shot span { position: absolute; left: 0; right: 0; bottom: 0; padding: 6px 10px; background: linear-gradient(transparent, rgb(0 0 0 / 0.75)); font-size: 13px; text-align: left; }

  .vbar { display: flex; align-items: center; justify-content: space-between; margin-bottom: 10px; }
  .toggle { display: flex; align-items: center; gap: 10px; color: var(--text-2); }
  .switch { width: 40px; height: 22px; border-radius: 999px; border: 0; padding: 0; position: relative; background: var(--switch-off); transition: background 0.15s; }
  .switch i { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform 0.15s; }
  .switch.on { background: var(--accent); }
  .switch.on i { transform: translateX(18px); }
  .vtable { background: var(--panel); border-radius: 8px; }
  .vrow { display: grid; grid-template-columns: 50px 2.4fr 110px 1fr 1fr 110px 40px; gap: 12px; padding: 0 16px; height: 52px; border-bottom: 1px solid var(--row-line); align-items: center; font-size: 13.5px; }
  .vrow.dim { opacity: 0.55; }
  .vhead { height: 40px; color: var(--muted); font-weight: 600; font-size: 12.5px; background: var(--panel-2); border-radius: 8px 8px 0 0; }
  .ellipsis { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .vt { display: inline-grid; place-items: center; width: 22px; height: 22px; border-radius: 4px; font-weight: 700; font-size: 12px; color: #fff; }
  .vt.release { background: #1f9d55; }
  .vt.beta { background: #c08a1e; }
  .vt.alpha { background: #c0392b; }
  .installed { font-weight: 600; }
  .vinstall { background: none; border: 1px solid var(--text-2); border-radius: 3px; padding: 6px 18px; font-weight: 600; }
  .vinstall:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .cl { background: none; border: 0; color: var(--muted); padding: 6px; border-radius: 50%; display: grid; place-items: center; }
  .cl:hover { color: var(--text); background: var(--panel-2); }

  .modal-back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.65); display: grid; place-items: center; z-index: 30; }
  .modal { position: relative; width: min(560px, 92vw); max-height: 80vh; display: flex; flex-direction: column; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 22px 24px 0; outline: none; }
  .modal h3 { margin: 0; font-size: 16px; padding-right: 20px; word-break: break-all; }
  .mx { position: absolute; top: 8px; right: 8px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  .modal-body { flex: 1; overflow-y: auto; margin: 10px 0 0; padding-bottom: 14px; max-width: none; }
  .modal-nav { display: flex; justify-content: space-between; margin: 0 -24px; padding: 10px 16px; background: var(--line); border-radius: 0 0 8px 8px; }
  .modal-nav button { background: none; border: 0; padding: 6px 8px; color: var(--text); }
  .modal-nav button:disabled { color: transparent; }
  .lbview { position: relative; overflow: hidden; border-radius: 8px; max-width: 90vw; max-height: 82vh; touch-action: none; }
  .lbview.zoomed { cursor: grab; }
  .lb { display: block; max-width: 90vw; max-height: 82vh; transform-origin: 0 0; transition: transform 0.08s ease-out; user-select: none; }
  .lbzoom { display: flex; gap: 4px; }
  .lbzoom button { min-width: 36px; height: 30px; padding: 0 10px; border: 0; border-radius: 6px; background: rgb(0 0 0 / 0.55); color: #fff; font-size: 15px; cursor: pointer; }
  .lbzoom button:hover { background: rgb(0 0 0 / 0.8); }
  .lbwrap { margin: 0; display: flex; flex-direction: column; align-items: center; gap: 8px; }
  .lbwrap figcaption { color: #fff; font-size: 13px; text-shadow: 0 1px 3px #000; }
  .lbnav { position: fixed; top: 50%; transform: translateY(-50%); width: 48px; height: 72px; border: 0; border-radius: 8px;
    background: rgb(0 0 0 / 0.45); color: #fff; font-size: 40px; line-height: 1; cursor: pointer; }
  .lbnav:hover { background: rgb(0 0 0 / 0.7); }
  .lbnav.prev { left: 16px; }
  .lbnav.next { right: 16px; }
</style>
