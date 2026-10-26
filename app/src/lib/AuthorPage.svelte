<script lang="ts">
  import { api, compact, ago, versionsLabel } from "./api";
  import { app, pushOverlay, popOverlay, closeOverlays } from "./state.svelte";
  import { cap, loadersOf, projectToHit } from "./install";
  import InstallButton from "./InstallButton.svelte";
  import { cfToHit } from "./cf";

  // Everything the author made for Minecraft, whatever game version it's for (the install button
  // says when a project doesn't fit the profile). Modrinth authors by name, CurseForge by id.
  let { name, cfId }: { name: string; cfId?: number } = $props();

  let user = $state<any>(null);
  let projects = $state<ReturnType<typeof projectToHit>[]>([]);
  let error = $state("");
  let sort = $state<"downloads" | "updated" | "name">("downloads");
  let typeFilter = $state("all");

  $effect(() => {
    const done = () => { if (projects.some((p) => p.project_type === "mod")) typeFilter = "mod"; };
    if (cfId) {
      api.cfAuthor(cfId).then((list) => {
        // CurseForge has no profile call; each project's author entry carries the picture.
        const me = (p: any) => p?.authors?.find((a: any) => a.id === cfId);
        const avatar = list.map(me).find((a: any) => a?.avatarUrl)?.avatarUrl;
        user = { username: name, cf: true, avatar_url: avatar };
        if (!avatar && list[0]) {
          api.cfProject(list[0].id).then(({ project }) => { if (user?.cf) user.avatar_url = me(project)?.avatarUrl; }).catch(() => {});
        }
        projects = list.map(cfToHit);
        done();
      }).catch((e) => (error = String(e)));
      return;
    }
    api.user(name).then((r) => {
      user = r.user;
      projects = r.projects.map((p) => projectToHit(p, r.user.username));
      done();
    }).catch((e) => (error = String(e)));
  });

  let totalDownloads = $derived(projects.reduce((n, p) => n + p.downloads, 0));
  let totalFollows = $derived(projects.reduce((n, p) => n + p.follows, 0));
  let types = $derived([...new Set(projects.map((p) => p.project_type))]);
  let shown = $derived(
    projects
      .filter((p) => typeFilter === "all" || p.project_type === typeFilter)
      .sort((a, b) => sort === "name" ? a.title.localeCompare(b.title)
        : sort === "updated" ? Date.parse(b.date_modified) - Date.parse(a.date_modified)
        : b.downloads - a.downloads),
  );
</script>

<div class="page">
  <div class="crumbs">
    <button class="back" onclick={popOverlay}>‹ Back</button>
    {#if app.target}<span class="target">Installing to <strong>{app.target.name}</strong></span>{/if}
    <button class="x" onclick={closeOverlays} aria-label="Close">✕</button>
  </div>

  {#if error}<div class="error">{error}</div>{/if}

  <header class="head">
    <div class="avatar">{#if user?.avatar_url}<img src={user.avatar_url} alt="" />{/if}</div>
    <div>
      <h2>{user?.username ?? name} <span class="role">Author</span></h2>
      <p class="muted">{compact(totalFollows)} followers · {compact(totalDownloads)} downloads</p>
      {#if user?.cf}<p class="muted">On CurseForge · every Minecraft project, all game versions</p>
      {:else if user}<p class="muted">Member since {new Date(user.created).toLocaleDateString()}{user.bio ? ` · ${user.bio}` : ""}</p>{/if}
    </div>
  </header>

  <div class="bar">
    <span class="count">Projects ({projects.length})</span>
    <div class="spacer"></div>
    <select class="select" bind:value={typeFilter} aria-label="Project type">
      <option value="all">All types</option>
      {#each types as t (t)}<option value={t}>{cap(t)}s</option>{/each}
    </select>
    <select class="select" bind:value={sort} aria-label="Sort">
      <option value="downloads">Most downloaded</option>
      <option value="updated">Recently updated</option>
      <option value="name">Name</option>
    </select>
  </div>

  <div class="list">
    {#each shown as h (h.project_id)}
      <div class="card">
        <button class="icon" onclick={() => pushOverlay({ kind: "project", hit: h })} aria-label="Open {h.title}">
          {#if h.icon_url}<img src={h.icon_url} alt="" loading="lazy" />{/if}
        </button>
        <div class="body">
          <button class="title" onclick={() => pushOverlay({ kind: "project", hit: h })}>{h.title}</button>
          <p class="desc">{h.description}</p>
          <div class="meta">
            <span class="chip">{cap(h.project_type)}</span>
            {#each (h.display_categories ?? []).slice(0, 3) as c (c)}<span class="chip plain">{cap(c)}</span>{/each}
            <span class="stats">
              <span>⤓ {compact(h.downloads)}</span><span>◷ {ago(h.date_modified)}</span>
              <span>{versionsLabel(h.versions)}</span>
              {#if loadersOf(h).length}<span>{cap(loadersOf(h)[0])}{loadersOf(h).length > 1 ? ` + ${loadersOf(h).length - 1}` : ""}</span>{/if}
            </span>
          </div>
        </div>
        <div class="act"><InstallButton hit={h} /></div>
      </div>
    {:else}
      {#if !error}<div class="muted pad">Loading…</div>{/if}
    {/each}
  </div>
</div>

<style>
  .page { position: absolute; inset: 0; background: var(--bg); z-index: 15; display: flex; flex-direction: column; padding: 0 28px; overflow: hidden; animation: fade 0.12s ease-out; }
  @keyframes fade { from { opacity: 0; transform: translateY(6px); } }
  .crumbs { display: flex; align-items: center; gap: 14px; padding: 14px 0 10px; color: var(--muted); }
  .back { background: none; border: 0; color: var(--muted); padding: 4px 8px; border-radius: 6px; }
  .back:hover, .x:hover { color: var(--text); background: var(--panel); }
  .target strong { color: var(--text); }
  .x { margin-left: auto; background: none; border: 0; font-size: 18px; color: var(--muted); padding: 4px 8px; border-radius: 6px; }
  .error { background: var(--err-bg); color: var(--err-text); padding: 8px 12px; border-radius: 6px; }
  .head { display: flex; gap: 18px; align-items: center; padding: 6px 0 16px; }
  .avatar { width: 96px; height: 96px; border-radius: 50%; overflow: hidden; background: var(--panel-2); border: 3px solid var(--accent); flex: none; }
  .avatar img { width: 100%; height: 100%; object-fit: cover; }
  h2 { margin: 0 0 4px; font-size: 24px; display: flex; align-items: center; gap: 10px; }
  .role { font-size: 12px; font-weight: 600; background: var(--panel-2); padding: 2px 8px; border-radius: 4px; color: var(--muted); }
  .muted { color: var(--muted); margin: 2px 0; }
  .pad { padding: 20px; }
  .bar { display: flex; align-items: center; gap: 10px; border-bottom: 1px solid var(--line); padding-bottom: 10px; }
  .count { font-size: 16px; font-weight: 600; border-bottom: 2px solid var(--accent); padding-bottom: 8px; margin-bottom: -11px; }
  .spacer { flex: 1; }
  .list { flex: 1; overflow-y: auto; min-height: 0; padding: 12px 0 30px; display: flex; flex-direction: column; gap: 8px; }
  .card { display: grid; grid-template-columns: 96px 1fr auto; gap: 16px; background: var(--panel); border-radius: 8px; padding: 10px 14px 10px 10px; content-visibility: auto; contain-intrinsic-size: auto 116px; }
  .icon { width: 96px; height: 96px; padding: 0; border: 0; border-radius: 6px; overflow: hidden; background: var(--panel-2); }
  .icon img { width: 100%; height: 100%; object-fit: cover; }
  .body { min-width: 0; display: flex; flex-direction: column; gap: 5px; justify-content: center; }
  .title { background: none; border: 0; padding: 0; font-size: 17px; font-weight: 600; text-align: left; }
  .title:hover { color: var(--accent); }
  .desc { margin: 0; color: var(--text-2); font-size: 13.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .meta { display: flex; align-items: center; gap: 6px; border-top: 1px solid var(--line); padding-top: 6px; }
  .chip { font-size: 12px; color: var(--text-2); border: 1px solid var(--line); border-radius: 4px; padding: 1px 6px; }
  .chip.plain { border: 0; }
  .stats { margin-left: auto; display: flex; gap: 14px; color: var(--muted); font-size: 12.5px; white-space: nowrap; }
  .act { display: flex; align-items: flex-start; }
</style>
