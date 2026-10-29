<script lang="ts">
  import { api, type Hit } from "./api";
  import { app, toast, toastError } from "./state.svelte";
  import { compat, install, isInstalled, refreshInstalled } from "./install";
  import { cached, recheck, updateState } from "./updates.svelte";
  import { settings } from "./settings.svelte";

  let { hit, big = false }: { hit: Hit; big?: boolean } = $props();
  let busy = $state(false);

  let c = $derived(compat(hit, app.target));
  let has = $derived(!!app.target && !!isInstalled(hit));
  /** The installed file's pending update, from the profile's last update check. */
  let upd = $derived.by(() => {
    void updateState.tick;
    const mine = app.target && isInstalled(hit);
    if (!mine) return null;
    return cached(app.target!.id)?.infos.find((f) => f.fileName === mine.fileName && f.update) ?? null;
  });

  async function update() {
    if (!upd || !app.target) return;
    const id = app.target.id;
    busy = true;
    try {
      const job = await api.applyUpdate(id, upd);
      if (job) toast(`${upd.update!.fileName} must be downloaded from curseforge.com: use Update in the profile's mods list.`);
      else toast(`Updated to ${upd.update!.name || upd.update!.fileName}.`);
    } catch (e) {
      toastError(e);
    }
    await refreshInstalled();
    recheck(id).catch(() => {});
    busy = false;
  }

  async function go(loaderOverride?: string) {
    busy = true;
    await install({ hit, loaderOverride });
    busy = false;
  }
</script>

{#if has && upd && !busy}
  <button class="btn" class:big title="Installed: {upd.fileName}. Newer: {upd.update?.name || upd.update?.fileName}" onclick={update}>Update</button>
{:else if has && busy}
  <button class="btn" class:big disabled>Updating…</button>
{:else if has && hit.project_type !== "datapack"}
  <span class="state" class:big>Installed</span>
{:else if busy}
  <button class="btn" class:big disabled>Installing…</button>
{:else if !c.ok && c.connector && settings.connector}
  <div class="pair">
    <span class="why" title="No NeoForge build">{c.reason}</span>
    <button class="btn alt" class:big onclick={() => (app.connector = { hit })}>Via Connector</button>
  </div>
{:else if !c.ok}
  <span class="why" class:big title="Not compatible with {app.target?.name}">{c.reason}</span>
{:else}
  <button class="btn" class:big onclick={() => go()}>Install</button>
{/if}

<style>
  .btn { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 7px 18px; font-weight: 600; white-space: nowrap; }
  .btn:hover:not(:disabled) { background: var(--accent-hover); }
  .btn:disabled { opacity: 0.6; }
  .btn.alt { background: none; border: 1px solid #f0a35e; color: #f0a35e; }
  .btn.alt:hover { background: rgb(240 163 94 / 0.1); }
  .big { padding: 9px 24px; font-size: 15px; }
  .state { font-weight: 600; padding: 7px 10px; white-space: nowrap; }
  .why { color: #f0a35e; font-size: 12.5px; white-space: nowrap; padding: 7px 4px; }
  .pair { display: flex; flex-direction: column; align-items: flex-end; gap: 2px; }
  .pair .why { padding: 0; }
</style>
