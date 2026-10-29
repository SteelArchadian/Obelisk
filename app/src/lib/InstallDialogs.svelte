<script lang="ts">
  import { api, ago, initials, type Instance, type World } from "./api";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { app } from "./state.svelte";
  import { compat, connectorStatus, install, setTarget } from "./install";

  // ---- profile picker ----
  let filter = $state("");
  let choice = $state<Instance | null>(null);
  let busy = $state(false);
  let shown = $derived(app.instances.filter((i) => i.name.toLowerCase().includes(filter.trim().toLowerCase())));

  async function confirmPick() {
    if (!app.picker || !choice) return;
    const pending = app.picker;
    busy = true;
    await setTarget(choice);
    app.picker = null;
    busy = false;
    const c = compat(pending.hit, choice);
    if (!c.ok && !pending.versionId && !pending.loaderOverride) {
      if (c.connector) app.connector = pending;
      return; // the page now shows why it can't go in this profile
    }
    await install(pending);
  }

  // ---- world picker (data packs) ----
  let worlds = $state<World[] | null>(null);
  let world = $state("");
  $effect(() => {
    worlds = null;
    world = "";
    const t = app.target;
    if (app.worldPicker && t) api.worlds(t.id).then((w) => { worlds = w; world = w[0]?.folder ?? ""; });
  });

  async function confirmWorld() {
    if (!app.worldPicker || !world) return;
    const pending = app.worldPicker;
    busy = true;
    app.worldPicker = null;
    await install({ ...pending, world });
    busy = false;
  }

  // ---- Connector confirmation ----
  let status = $state<{ connector: boolean; ffapi: boolean; extras: boolean } | null>(null);
  let addMissing = $state(true);

  $effect(() => {
    status = null;
    if (app.connector && app.target) connectorStatus(app.target).then((s) => (status = s));
  });

  async function confirmConnector() {
    if (!app.connector || !app.target || !status) return;
    const pending = app.connector;
    busy = true;
    if (addMissing) {
      for (const [ok, slug] of [[status.connector, "connector"], [status.ffapi, "forgified-fabric-api"]] as const) {
        if (!ok) {
          const p = await api.project(slug);
          await install({ hit: { ...pending.hit, project_id: p.id, title: p.title, project_type: "mod" } });
        }
      }
    }
    app.connector = null;
    await install({ ...pending, loaderOverride: "fabric" });
    busy = false;
  }
</script>

{#if app.picker}
  <div class="back" role="presentation" onclick={() => (app.picker = null)}>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.picker = null)}>
      <button class="x" onclick={() => (app.picker = null)} aria-label="Close">✕</button>
      <h3>Install {app.picker.hit.title}</h3>
      <input type="search" class="search wide" placeholder="Search for profile" bind:value={filter} />
      <div class="list">
        {#each shown as inst (inst.id)}
          {@const c = compat(app.picker.hit, inst)}
          <label class="opt" class:dim={!c.ok && !c.connector}>
            <input type="radio" name="profile" checked={choice?.id === inst.id} onchange={() => (choice = inst)} />
            <span class="pic">{#if app.icons[inst.id]}<img src={app.icons[inst.id]} alt="" />{:else}{initials(inst.name)}{/if}</span>
            <span class="txt"><strong>{inst.name}</strong><small>{inst.loader} · {inst.minecraft}{#if !c.ok} · <em>{c.reason}{c.connector ? " (Connector possible)" : ""}</em>{/if}</small></span>
          </label>
        {/each}
      </div>
      <p class="hint">Obelisk remembers this profile for the rest of this browsing session. Change it any time from the “Installing to” chip.</p>
      <div class="actions">
        <button class="primary" disabled={!choice || busy} onclick={confirmPick}>{busy ? "Working…" : "Install"}</button>
      </div>
    </div>
  </div>
{/if}

{#if app.worldPicker && app.target}
  <div class="back" role="presentation" onclick={() => (app.worldPicker = null)}>
    <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.worldPicker = null)}>
      <button class="x" onclick={() => (app.worldPicker = null)} aria-label="Close">✕</button>
      <h3>Add {app.worldPicker.hit.title} to a world</h3>
      <p class="hint">Data packs belong to one world. Pick a world in {app.target.name}.</p>
      <div class="list">
        {#each worlds ?? [] as w (w.folder)}
          <label class="opt">
            <input type="radio" name="world" checked={world === w.folder} onchange={() => (world = w.folder)} />
            <span class="pic">{#if w.icon}<img src={convertFileSrc(w.icon)} alt="" />{:else}{initials(w.folder)}{/if}</span>
            <span class="txt"><strong>{w.folder}</strong><small>Played {ago(w.lastPlayedMs).toLowerCase()} · {w.datapacks.length} data pack{w.datapacks.length === 1 ? "" : "s"}</small></span>
          </label>
        {:else}
          <p class="muted">{worlds ? `No worlds in ${app.target.name} yet. Create one in game first, then add data packs to it.` : "Loading worlds…"}</p>
        {/each}
      </div>
      <div class="actions">
        <button class="primary" disabled={!world || busy} onclick={confirmWorld}>{busy ? "Installing…" : "Install"}</button>
      </div>
    </div>
  </div>
{/if}

{#if app.connector && app.target}
  <div class="back" role="presentation" onclick={() => (app.connector = null)}>
    <div class="dialog small" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (app.connector = null)}>
      <button class="x" onclick={() => (app.connector = null)} aria-label="Close">✕</button>
      <h3>Install the Fabric build on NeoForge?</h3>
      <p><strong>{app.connector.hit.title}</strong> has no NeoForge build. Sinytra Connector can run Fabric mods on NeoForge, but it doesn't work with every mod. Test in a copy before your main world.</p>
      {#if status}
        <ul class="checks">
          <li class:ok={status.connector}>{status.connector ? "✓" : "✗"} Sinytra Connector</li>
          <li class:ok={status.ffapi}>{status.ffapi ? "✓" : "✗"} Forgified Fabric API (replaces Fabric API)</li>
          <li class:ok={status.extras}>{status.extras ? "✓" : "–"} Connector Extras (needed by some mods)</li>
        </ul>
        {#if !status.connector || !status.ffapi}
          <label class="check"><input type="checkbox" bind:checked={addMissing} /> Also install what's missing</label>
        {/if}
      {:else}<p class="muted">Checking {app.target.name}…</p>{/if}
      <div class="actions">
        <button class="ghost" onclick={() => (app.connector = null)}>Cancel</button>
        <button class="primary" disabled={!status || busy} onclick={confirmConnector}>{busy ? "Installing…" : "Install via Connector"}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(520px, 92vw); height: min(640px, 88vh); background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 12px; outline: none; }
  .dialog.small { height: auto; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); font-size: 15px; padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  .wide { width: 100%; }
  .list { flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; }
  .opt { display: flex; align-items: center; gap: 12px; padding: 8px; border-radius: 6px; cursor: pointer; }
  .opt:hover { background: var(--panel); }
  .opt.dim { opacity: 0.55; }
  .opt input { accent-color: var(--accent); width: 18px; height: 18px; }
  .pic { width: 40px; height: 40px; border-radius: 4px; overflow: hidden; background: var(--panel); display: grid; place-items: center; font-weight: 700; color: var(--muted); }
  .pic img { width: 100%; height: 100%; object-fit: cover; }
  .txt { display: flex; flex-direction: column; }
  .txt small { color: var(--muted); }
  .txt em { color: #f0a35e; font-style: normal; }
  .hint { color: var(--muted); font-size: 12.5px; margin: 0; }
  .actions { display: flex; justify-content: flex-end; gap: 10px; border-top: 1px solid var(--line); padding-top: 12px; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 8px 22px; font-weight: 600; }
  .primary:disabled { opacity: 0.5; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 8px 16px; }
  p { margin: 0; color: var(--text-2); line-height: 1.5; }
  .checks { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 4px; color: #f0a35e; }
  .checks li.ok { color: var(--mr); }
  .check { display: flex; gap: 8px; align-items: center; }
  .muted { color: var(--muted); }
</style>
