<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { api, ago, initials, type Instance } from "./api";
  import { app, openBrowse, toast } from "./state.svelte";
  import { setTarget } from "./install";
  import { openPackMenu } from "./PackMenu.svelte";
  import { syncInstances } from "./install";
  import { settings } from "./settings.svelte";
  import { kill, play, progress, running, starting } from "./play.svelte";
  import { checkAllDupes, dupeState, flagged } from "./dupes.svelte";
  import CollabBadge from "./CollabBadge.svelte";

  // Stop on a playing tile asks once more (it force-closes the game), then closes it.
  let stopArmed = $state<string | null>(null);
  let stopTimer: ReturnType<typeof setTimeout> | undefined;
  function stop(inst: Instance) {
    clearTimeout(stopTimer);
    if (stopArmed === inst.id) { stopArmed = null; kill(inst); return; }
    stopArmed = inst.id;
    stopTimer = setTimeout(() => (stopArmed = null), 3000);
  }

  // Duplicate mods: checked for every profile when the library shows, and again when Obelisk regains focus.
  $effect(() => {
    const ids = app.instances.map((i) => i.id);
    checkAllDupes(ids);
    const onFocus = () => checkAllDupes(ids);
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  });

  let joining = $state<string | null>(null);
  let joinBusy = $state(false);
  async function join() {
    if (!joining?.trim()) return;
    joinBusy = true;
    try {
      const id = await invoke<string>("share_join", { code: joining.trim() });
      await syncInstances();
      joining = null;
      toast(`Joined! “${id}” is in your modpacks.`);
      const inst = app.instances.find((i) => i.id === id);
      if (inst) openInstance(inst);
    } catch (e) { toast(String(e)); }
    joinBusy = false;
  }

  type TrashedPack = { id: string; instanceId: string; name: string; deletedMs: number; bytes: number };
  let trash = $state<TrashedPack[] | null>(null);
  let forget = $state<TrashedPack | null>(null);
  const loadTrash = () => invoke<TrashedPack[]>("pack_trash").then((t) => (trash = t)).catch((e) => toast(String(e)));
  async function restorePack(t: TrashedPack) {
    try {
      const id = await invoke<string>("restore_pack", { id: t.id });
      await syncInstances();
      toast(id === t.instanceId ? `${t.name} is back.` : `${t.name} is back as “${id}”.`);
      await loadTrash();
    } catch (e) { toast(String(e)); }
  }
  async function removePack(t: TrashedPack) {
    try {
      await invoke("remove_pack", { id: t.id });
      forget = null;
      await loadTrash();
    } catch (e) { toast(String(e)); }
  }
  const gb = (n: number) => (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : `${Math.max(1, Math.round(n / (1 << 20)))} MB`);
  const daysLeft = (ms: number) => Math.max(0, Math.ceil((ms + settings.packTrashHours * 3_600_000 - Date.now()) / 86_400_000));

  type Sort = "played" | "most" | "name" | "version";
  const SORTS: [Sort, string][] = [["played", "Recently played"], ["most", "Most played"], ["name", "Name"], ["version", "Game version"]];
  const SIZES: [number, string][] = [[150, "S"], [190, "M"], [230, "L"], [280, "XL"]];

  const saved = (k: string, d: string) => { try { return localStorage.getItem(k) ?? d; } catch { return d; } };
  let filter = $state("");
  let searching = $state(false);
  let sort = $state<Sort>(saved("obelisk.libSort", "played") as Sort);
  let asc = $state(saved("obelisk.libAsc", "0") === "1");
  let kind = $state<"all" | "modded" | "vanilla">("all");
  let view = $state<"tiles" | "list">(saved("obelisk.libView", "tiles") as "tiles" | "list");
  let size = $state(Number(saved("obelisk.tileSize", "190")));
  let menu = $state<"" | "sort" | "filter" | "view" | "size">("");

  let sizeLabel = $derived(SIZES.reduce((best, s) => (Math.abs(s[0] - size) < Math.abs(best[0] - size) ? s : best))[1]);
  let shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    let list = app.instances.filter((i) =>
      (!q || i.name.toLowerCase().includes(q)) &&
      (kind === "all" || (kind === "vanilla") === (i.loader === "Vanilla")));
    const cmp: Record<Sort, (a: Instance, b: Instance) => number> = {
      played: (a, b) => b.lastLaunchMs - a.lastLaunchMs,
      most: (a, b) => b.secondsPlayed - a.secondsPlayed,
      name: (a, b) => a.name.localeCompare(b.name),
      version: (a, b) => b.minecraft.localeCompare(a.minecraft, undefined, { numeric: true }),
    };
    list = [...list].sort(cmp[sort]);
    return asc ? list.reverse() : list;
  });

  $effect(() => {
    try {
      localStorage.setItem("obelisk.tileSize", String(size));
      localStorage.setItem("obelisk.libSort", sort);
      localStorage.setItem("obelisk.libAsc", asc ? "1" : "0");
      localStorage.setItem("obelisk.libView", view);
    } catch { /* ignore */ }
  });

  function openInstance(inst: Instance) {
    app.current = inst;
    app.page = "instance";
  }


  async function createProfile() {
    try {
      await invoke("open_prism");
      toast("Use Add Instance in Prism. The new profile shows up here when you come back.");
    } catch (e) {
      toast(String(e));
    }
  }

  function addContent(inst: Instance) {
    setTarget(inst);
    app.current = inst;
    openBrowse("mod");
  }

  const onContext = openPackMenu;

  const toggle = (m: typeof menu) => (e: MouseEvent) => { e.stopPropagation(); menu = menu === m ? "" : m; };
</script>

<svelte:window onclick={() => (menu = "")} />

<div class="library">
  <div class="toolbar">
    <button class="icon" title="Create a new profile (opens Prism's New Instance window)" aria-label="Create a profile" onclick={createProfile}>
      <svg viewBox="0 0 24 24" width="18" height="18"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" /></svg>
    </button>
    <button class="icon" title="Import a modpack: a friend's share code, or a .zip / .mrpack file" aria-label="Import a modpack" onclick={() => (app.packImport = "")}>
      <svg viewBox="0 0 24 24" width="18" height="18"><path d="M12 4v11m0 0-4-4m4 4 4-4M5 19h14" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </button>
    <button class="icon" title="Join a friend's shared pack (paste their invite code)" aria-label="Join a shared pack" onclick={() => (joining = joining === null ? "" : null)}>
      <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><circle cx="9" cy="8" r="3.2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M3 19c.8-3.2 3.2-5 6-5s5.2 1.8 6 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /><path d="M18 8v6M15 11h6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" /></svg>
    </button>
    <button class="icon" title="Sync profiles from CurseForge and other launchers" aria-label="Sync profiles" onclick={() => { app.settingsSection = "sync"; app.page = "settings"; }}>
      <svg viewBox="0 0 24 24" width="18" height="18"><path d="M20 12a8 8 0 0 1-14.3 4.9M4 12a8 8 0 0 1 14.3-4.9M18 3v4h-4M6 21v-4h4" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </button>
    <div class="spacer"></div>

    {#if searching}
      <!-- svelte-ignore a11y_autofocus -->
      <input class="search" placeholder="Search profiles…" bind:value={filter} autofocus onblur={() => !filter && (searching = false)} />
    {:else}
      <button class="icon" title="Search" onclick={() => (searching = true)}>
        <svg viewBox="0 0 24 24" width="17" height="17"><circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="2" /><path d="m20 20-3.5-3.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
      </button>
    {/if}

    <div class="pop">
      <button class="icon" class:on={menu === "sort"} title="Sort" onclick={toggle("sort")}>⇅</button>
      {#if menu === "sort"}
        <div class="menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
          {#each SORTS as [id, label] (id)}
            <button class:on={sort === id} onclick={() => (sort === id ? (asc = !asc) : ((sort = id), (asc = false)))}>
              {label}{#if sort === id}<span>{asc ? "↑" : "↓"}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
    </div>

    <div class="pop">
      <button class="icon" class:on={menu === "filter" || kind !== "all"} title="Filter" onclick={toggle("filter")}>
        <svg viewBox="0 0 24 24" width="16" height="16"><path d="M3 5h18l-7 8v6l-4-2v-4z" fill="currentColor" /></svg>
      </button>
      {#if menu === "filter"}
        <div class="menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
          {#each [["all", "All modpacks"], ["modded", "Modded"], ["vanilla", "Vanilla"]] as [id, label] (id)}
            <button class:on={kind === id} onclick={() => { kind = id as any; menu = ""; }}>{label}{#if kind === id}<span>✓</span>{/if}</button>
          {/each}
        </div>
      {/if}
    </div>

    {#if view === "tiles"}
      <!-- Tile size: click to open; stays while hovered, closes when the pointer leaves. -->
      <div class="pop" role="group" onmouseleave={() => menu === "size" && (menu = "")}>
        <button class="icon size" class:on={menu === "size"} title="Tile size" onclick={toggle("size")}><span class="sz">{sizeLabel}</span></button>
        {#if menu === "size"}
          <div class="menu slider-pop" role="dialog" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
            <input type="range" min="130" max="300" step="5" bind:value={size} aria-label="Tile size" />
          </div>
        {/if}
      </div>
    {/if}

    <button class="icon" class:on={trash !== null} title="Modpack trash: deleted modpacks" aria-label="Modpack trash"
      onclick={() => (trash === null ? loadTrash() : (trash = null))}>
      <svg viewBox="0 0 24 24" width="17" height="17" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
    </button>

    <div class="pop">
      <button class="icon" class:on={menu === "view"} title="View" onclick={toggle("view")}>
        {#if view === "tiles"}
          <!-- Four outlined tiles, the outer corners rounded. -->
          <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="round">
            <path d="M7 3.5h3.5v7h-7V7A3.5 3.5 0 0 1 7 3.5Z" /><path d="M13.5 3.5H17A3.5 3.5 0 0 1 20.5 7v3.5h-7Z" />
            <path d="M3.5 13.5h7v7H7A3.5 3.5 0 0 1 3.5 17Z" /><path d="M13.5 13.5h7V17a3.5 3.5 0 0 1-3.5 3.5h-3.5Z" />
          </svg>
        {:else}
          <svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M4.5 7h15M4.5 12h15M4.5 17h15" /></svg>
        {/if}
      </button>
      {#if menu === "view"}
        <div class="menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
          {#each [["tiles", "Tiles"], ["list", "List"]] as [id, label] (id)}
            <button class:on={view === id} onclick={() => { view = id as any; menu = ""; }}>{label}{#if view === id}<span>✓</span>{/if}</button>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  {#if joining !== null}
    <div class="join">
      <strong>Join a friend's shared pack</strong>
      <span class="muted">Paste the invite code they sent you. Obelisk creates the profile and downloads the pack straight from them (they need Obelisk open).</span>
      <div class="jrow">
        <input class="search jcode" placeholder="obelisk1.…" bind:value={joining} onkeydown={(e) => e.key === "Enter" && join()} />
        <button class="tool restore" disabled={joinBusy || !joining.trim()} onclick={join}>{joinBusy ? "Downloading the pack…" : "Join"}</button>
        <button class="tool" disabled={joinBusy} onclick={() => (joining = null)}>Cancel</button>
      </div>
    </div>
  {/if}

  {#if trash !== null}
    <div class="ptrash">
      <div class="phead"><strong>Modpack trash</strong><span class="muted">Deleted modpacks stay here for {Math.round(settings.packTrashHours / 24)} days (change it in Settings).</span>
        <div class="spacer"></div><button class="tool" onclick={() => (trash = null)}>← Back to my modpacks</button></div>
      {#each trash as t (t.id)}
        <div class="prow">
          <div class="pinfo"><strong>{t.name}</strong><small>Deleted {ago(t.deletedMs).toLowerCase()} · {gb(t.bytes)} · deleted for good in {daysLeft(t.deletedMs)} day{daysLeft(t.deletedMs) === 1 ? "" : "s"}</small></div>
          {#if forget?.id === t.id}
            <span class="muted">Delete for good? It goes to the Recycle Bin.</span>
            <button class="tool" onclick={() => (forget = null)}>Cancel</button>
            <button class="tool danger" onclick={() => removePack(t)}>Delete</button>
          {:else}
            <button class="tool" onclick={() => (forget = t)}>Delete forever</button>
            <button class="tool restore" onclick={() => restorePack(t)}>Restore</button>
          {/if}
        </div>
      {:else}
        <p class="muted">The modpack trash is empty.</p>
      {/each}
    </div>
  {:else if view === "tiles"}
    <div class="grid" style:--tile="{size}px">
      {#each shown as inst (inst.id)}
        <div class="tile" class:playing={running[inst.id]} role="group" oncontextmenu={(e) => onContext(e, inst)}>
          <div class="artwrap">
          <button class="art" onclick={() => openInstance(inst)} title="Open {inst.name}">
            {#if app.icons[inst.id]}<img src={app.icons[inst.id]} alt="" draggable="false" />{:else}<span>{initials(inst.name)}</span>{/if}
            <span class="badge">{inst.loader} · {inst.minecraft}</span>
            <CollabBadge id={inst.id} />
          </button>
          {#if flagged(inst.id)}
            <button class="dupe-stripe" onclick={() => openInstance(inst)} title="Open {inst.name} to choose which copy to remove">
              ⚠ {dupeState[inst.id].dupes.length === 1 ? "Duplicate mod" : `${dupeState[inst.id].dupes.length} duplicate mods`}: fix before playing
            </button>
          {/if}
          {#if starting[inst.id]}
            {@const p = progress[inst.id]}
            <div class="prog" aria-live="polite">
              <small>{p?.text ?? "Starting…"}{#if p?.pct != null} · {p.pct}%{/if}</small>
              <div class="bar"><i class:indet={p?.pct == null} style:width={p?.pct != null ? `${p.pct}%` : null}></i></div>
            </div>
          {/if}
          </div>
          <div class="metarow">
            <button class="meta" onclick={() => openInstance(inst)}>
              <strong>{inst.name}</strong>{#if running[inst.id]}<small class="now"><i></i>Playing now</small>{:else}<small>Played {ago(inst.lastLaunchMs).toLowerCase()}</small>{/if}
            </button>
            <button class="tile-play" class:busy={starting[inst.id]} class:live={running[inst.id]} class:armed={stopArmed === inst.id}
              title={running[inst.id] ? (stopArmed === inst.id ? "Click again to close the game (unsaved progress since the last autosave is lost)" : `Stop ${inst.name}`) : starting[inst.id] ? "Starting…" : `Play ${inst.name}`}
              aria-label={running[inst.id] ? `Stop ${inst.name}` : `Play ${inst.name}`} onclick={() => (running[inst.id] ? stop(inst) : play(inst))}>
              {#if running[inst.id] && stopArmed === inst.id}<span class="sure">Stop?</span>
              {:else if running[inst.id]}<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><rect x="5" y="5" width="14" height="14" rx="1.5" fill="currentColor" /></svg><span class="sure">Stop</span>
              {:else if starting[inst.id]}<span class="spin"></span>{:else}<svg viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M8 5v14l11-7z" fill="currentColor" /></svg>{/if}
            </button>
          </div>
        </div>
      {:else}
        <div class="empty">No profiles yet. Use Sync & Import to bring in your CurseForge profiles, or Import a pack.</div>
      {/each}
    </div>
  {:else}
    <div class="rows">
      {#each shown as inst (inst.id)}
        <button class="row" onclick={() => openInstance(inst)} oncontextmenu={(e) => onContext(e, inst)}>
          <span class="pic">{#if app.icons[inst.id]}<img src={app.icons[inst.id]} alt="" draggable="false" />{:else}{initials(inst.name)}{/if}</span>
          <strong>{inst.name}{#if flagged(inst.id)} <span class="dupe-tag">⚠ duplicate mods</span>{/if}</strong>
          <span class="muted">{inst.loader} {inst.loaderVersion}</span>
          <span class="muted">{inst.minecraft}</span>
          <span class="muted">{Math.round(inst.secondsPlayed / 3600)} h played</span>
          <span class="muted">{ago(inst.lastLaunchMs)}</span>
        </button>
      {/each}
    </div>
  {/if}
</div>


<style>
  .library { display: flex; flex-direction: column; flex: 1; min-height: 0; }
  .toolbar { display: flex; align-items: center; gap: 6px; padding: 12px 0; }
  .spacer { flex: 1; }
  .icon { width: 36px; height: 36px; display: grid; place-items: center; background: var(--panel); border: 1px solid var(--line); border-radius: 7px; color: var(--text); font-size: 16px; box-shadow: 0 2px 8px rgb(0 0 0 / 0.3); transition: background 0.12s, border-color 0.12s; }
  .icon:hover, .icon.on { background: var(--panel-2); border-color: var(--accent); color: var(--text); }
  .icon.size { font-size: 11px; font-weight: 800; }
  .sz { display: grid; place-items: center; min-width: 18px; height: 18px; padding: 0 3px; border: 1.8px solid currentColor; border-radius: 4px; font: 800 10.5px/1 Nunito, system-ui, sans-serif; letter-spacing: 0; }
  .pop { position: relative; }
  .join { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; margin-bottom: 12px; background: var(--panel); border: 1px solid var(--accent); border-radius: 8px; }
  .join .muted { color: var(--muted); font-size: 13px; }
  .jrow { display: flex; gap: 8px; }
  .jcode { flex: 1; width: auto; font-family: ui-monospace, Consolas, monospace; }
  .ptrash { display: flex; flex-direction: column; gap: 8px; overflow-y: auto; }
  .phead { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; padding-bottom: 6px; }
  .phead .muted, .ptrash > .muted { color: var(--muted); }
  .spacer { flex: 1; }
  .prow { display: flex; align-items: center; gap: 10px; padding: 10px 14px; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; }
  .pinfo { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .pinfo small { color: var(--muted); }
  .tool { background: var(--panel-2); border: 1px solid var(--line); border-radius: 6px; padding: 6px 12px; }
  .tool.restore { background: var(--accent); color: #fff; border-color: transparent; }
  .tool.danger { color: var(--danger); }
  .menu { position: absolute; right: 0; top: calc(100% + 4px); z-index: 20; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 4px; min-width: 180px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.45); }
  .menu > button { display: flex; justify-content: space-between; width: 100%; background: none; border: 0; padding: 7px 10px; border-radius: 5px; text-align: left; color: var(--text-2); }
  .menu > button:hover { background: var(--panel); }
  .menu > button.on { color: var(--text); font-weight: 600; }
  .slider-pop { min-width: 0; padding: 10px 12px; top: 100%; }
  .slider-pop input { width: 160px; accent-color: var(--accent); }

  .grid { display: grid; grid-template-columns: repeat(auto-fill, var(--tile)); gap: 12px; overflow-y: auto; padding-bottom: 20px; align-content: start; }
  .tile { position: relative; background: var(--panel); border-radius: 8px; overflow: hidden; transition: transform 0.15s, box-shadow 0.15s; }
  .tile:hover { transform: translateY(-2px); box-shadow: 0 6px 20px rgb(0 0 0 / 0.35); }
  .art { position: relative; display: grid; place-items: center; width: 100%; aspect-ratio: 1; padding: 0; border: 0; background: var(--panel-2); font-size: 32px; font-weight: 700; color: var(--muted); }
  /* Play straight from the library: bottom-right corner of the artwork. */
  .artwrap { position: relative; }
  .dupe-stripe { position: absolute; left: 0; right: 0; top: 50%; transform: translateY(-50%); padding: 8px 6px; border: 0; border-radius: 0;
    background: rgb(200 30 30 / 0.92); color: #fff; font-weight: 700; font-size: 12.5px; text-align: center; cursor: pointer;
    box-shadow: 0 2px 10px rgb(0 0 0 / 0.4); }
  .dupe-stripe:hover { background: rgb(220 35 35); }
  .dupe-tag { color: var(--err-text); background: var(--err-bg); font-size: 11px; font-weight: 700; padding: 1px 6px; border-radius: 4px; margin-left: 6px; }
  .prog { position: absolute; left: 0; right: 0; bottom: 0; display: flex; flex-direction: column; gap: 6px; padding: 18px 10px 9px; background: linear-gradient(transparent, rgb(0 0 0 / 0.8)); color: #fff; pointer-events: none; }
  .prog small { font-size: 11.5px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prog .bar { height: 5px; border-radius: 3px; background: rgb(255 255 255 / 0.25); overflow: hidden; }
  .prog i { display: block; height: 100%; background: var(--accent); transition: width 0.3s; }
  .prog i.indet { width: 35%; animation: indet 1.2s ease-in-out infinite; }
  @keyframes indet { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
  .metarow { display: flex; align-items: center; }
  .metarow .meta { flex: 1; min-width: 0; }
  .tile-play {
    flex: none; margin-right: 10px; width: 40px; height: 40px; border-radius: 50%;
    display: grid; place-items: center; border: 0; color: #fff; background: var(--accent);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.45); opacity: 0.92; transform: scale(0.94); transition: transform 0.12s, opacity 0.12s, filter 0.12s;
  }
  .tile:hover .tile-play { opacity: 1; transform: scale(1); }
  .tile-play:hover { filter: brightness(1.15); }
  .tile-play svg { margin-left: 2px; }
  .tile-play.busy, .tile-play.live { opacity: 1; transform: scale(1); }
  .tile-play.live { background: var(--danger); }
  .tile-play.live { width: auto; padding: 0 14px; border-radius: 20px; display: flex; gap: 6px; }
  .tile-play.live svg { margin-left: 0; }
  .sure { font-size: 13px; font-weight: 700; }
  .tile.playing { box-shadow: 0 0 0 2px var(--ok, #3fb950), 0 0 18px color-mix(in srgb, var(--ok, #3fb950) 45%, transparent); }
  .meta small.now { color: var(--ok, #3fb950); display: flex; align-items: center; gap: 6px; }
  .now i { width: 7px; height: 7px; border-radius: 50%; background: currentColor; animation: pulse 1.6s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .spin { width: 18px; height: 18px; border-radius: 50%; border: 2.5px solid rgb(255 255 255 / 0.35); border-top-color: #fff; animation: spin 0.8s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .art img { width: 100%; height: 100%; object-fit: cover; -webkit-user-drag: none; }
  .badge { position: absolute; top: 8px; right: 8px; font-size: 11.5px; font-weight: 600; color: #fff; background: rgb(0 0 0 / 0.65); padding: 3px 7px; border-radius: 4px; }
  .meta { display: flex; flex-direction: column; align-items: flex-start; width: 100%; padding: 10px 12px 12px; background: none; border: 0; text-align: left; }
  .meta strong { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%; }
  .meta small { color: var(--muted); }
  .rows { display: flex; flex-direction: column; gap: 4px; overflow-y: auto; }
  .row { display: grid; grid-template-columns: 40px 2fr 1.2fr 0.8fr 1fr 1fr; gap: 14px; align-items: center; padding: 8px 12px; background: var(--panel); border: 0; border-radius: 6px; text-align: left; }
  .row:hover { background: var(--panel-2); }
  .pic { width: 40px; height: 40px; border-radius: 4px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; font-weight: 700; color: var(--muted); }
  .pic img { width: 100%; height: 100%; object-fit: cover; }
  .muted { color: var(--muted); }
  .empty { grid-column: 1 / -1; color: var(--muted); padding: 40px; text-align: center; }
</style>
