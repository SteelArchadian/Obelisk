<script lang="ts">
  // A profile's backups: what changed since the last one, automatic backups (noon and midnight,
  // only when something changed), checkpoints you make yourself, and safety backups Obelisk takes
  // before risky actions. Each shows its changelog; restore the whole pack or undo one change.
  import { invoke } from "@tauri-apps/api/core";
  import { toast, toastError } from "./state.svelte";
  import { settings, saveSettings } from "./settings.svelte";
  import type { Instance } from "./api";

  type Change = { kind: "added" | "removed" | "changed"; area: string; path: string; label: string; details?: string[] };
  type Backup = { id: string; atMs: number; kind: "auto" | "manual" | "safety"; label: string; files: number; bytes: number; changes: Change[] };
  type State = { backups: Backup[]; pending: Change[]; diskBytes: number };

  let { inst }: { inst: Instance } = $props();

  let st = $state<State | null>(null);
  let tab = $state<"auto" | "manual">("auto");
  let open = $state<string | null>(null);
  let label = $state("");
  let busy = $state(false);
  let confirm = $state<Backup | null>(null);

  const load = () => invoke<State>("backups", { instanceId: inst.id }).then((s) => { if (!st && s.backups[0]?.changes.length) open = s.backups[0].id; st = s; }).catch((e) => toastError(e));
  $effect(() => { void inst.id; load(); });

  const shown = $derived((st?.backups ?? []).filter((b) => (tab === "manual" ? b.kind === "manual" : b.kind !== "manual")));
  const when = (ms: number) => new Date(ms).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  const size = (n: number) => (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : n >= 1 << 20 ? `${Math.round(n / (1 << 20))} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
  const AREA: Record<string, string> = { mods: "Mods", configs: "Configs", resourcepacks: "Resource packs", shaderpacks: "Shaders", scripts: "Scripts", loader: "Loader", other: "Other" };
  const VERB = { added: "Added", removed: "Removed", changed: "Changed" };

  function summary(changes: Change[]) {
    if (!changes.length) return "No changes";
    const by: Record<string, number> = {};
    for (const c of changes) by[c.area] = (by[c.area] ?? 0) + 1;
    const ONE: Record<string, string> = { mods: "mod", configs: "config", resourcepacks: "resource pack", shaderpacks: "shader", scripts: "script", loader: "loader change", other: "other file" };
    const MANY: Record<string, string> = { mods: "mods", configs: "configs", resourcepacks: "resource packs", shaderpacks: "shaders", scripts: "scripts", loader: "loader changes", other: "other files" };
    return Object.entries(by).map(([a, n]) => `${n} ${(n === 1 ? ONE[a] : MANY[a]) ?? a}`).join(", ");
  }
  function groups(changes: Change[]) {
    const g: Record<string, Change[]> = {};
    for (const c of changes) (g[c.area] ??= []).push(c);
    return Object.entries(g);
  }

  async function checkpoint() {
    busy = true;
    try {
      await invoke("create_checkpoint", { instanceId: inst.id, label });
      label = "";
      tab = "manual";
      toast("Checkpoint saved.");
      await load();
    } catch (e) { toastError(e); }
    busy = false;
  }

  async function restore(b: Backup) {
    busy = true;
    try {
      const changes = await invoke<Change[]>("restore_backup", { instanceId: inst.id, backupId: b.id });
      toast(`Restored to ${when(b.atMs)} (${changes.length} change${changes.length === 1 ? "" : "s"}). A safety backup of how it was is in Automatic.`);
      confirm = null;
      await load();
    } catch (e) { toastError(e); }
    busy = false;
  }

  async function undo(b: Backup, c: Change) {
    busy = true;
    try {
      await invoke("revert_change", { instanceId: inst.id, backupId: b.id, path: c.path });
      toast(`${VERB[c.kind]} ${c.label}: undone.`);
      await load();
    } catch (e) { toastError(e); }
    busy = false;
  }

  const UNDO = { added: "Remove", removed: "Bring back", changed: "Undo" };
</script>

<div class="backups">
  {#if !st}
    <p class="muted">Loading backups…</p>
  {:else}
    <section class="now">
      <div class="row">
        <strong>Since the last backup</strong>
        <span class="muted">{st.backups.length ? summary(st.pending) : "No backups yet"}</span>
        <div class="spacer"></div>
        <input class="search" placeholder="Checkpoint name (optional)" bind:value={label} onkeydown={(e) => e.key === "Enter" && checkpoint()} />
        <button class="primary" disabled={busy} onclick={checkpoint}>Create checkpoint</button>
      </div>
      {#if st.pending.length}
        <ul class="changes">
          {#each st.pending.slice(0, 8) as c (c.path)}<li><span class="k {c.kind}">{VERB[c.kind]}</span> {c.label}</li>{/each}
          {#if st.pending.length > 8}<li class="muted">…and {st.pending.length - 8} more</li>{/if}
        </ul>
      {/if}
    </section>

    <div class="tabs">
      <button class:on={tab === "auto"} onclick={() => (tab = "auto")}>Automatic ({st.backups.filter((b) => b.kind !== "manual").length})</button>
      <button class:on={tab === "manual"} onclick={() => (tab = "manual")}>Checkpoints ({st.backups.filter((b) => b.kind === "manual").length})</button>
      <div class="spacer"></div>
      <span class="muted small">Using {size(st.diskBytes)} · worlds aren't included</span>
    </div>

    <div class="list">
      {#each shown as b (b.id)}
        <div class="b" class:open={open === b.id}>
          <button class="head" onclick={() => (open = open === b.id ? null : b.id)}>
            <span class="caret">{open === b.id ? "▾" : "▸"}</span>
            <span class="when">{when(b.atMs)}</span>
            <span class="what">{b.kind === "manual" ? b.label : b.kind === "safety" ? b.label : b.label || "Automatic"}</span>
            <span class="muted small sum">{summary(b.changes)}</span>
          </button>
          {#if open === b.id}
            <div class="body">
              {#if b.changes.length}
                {#each groups(b.changes) as [area, list] (area)}
                  <h5>{AREA[area] ?? area}</h5>
                  <ul class="changes">
                    {#each list as c (c.path)}
                      <li>
                        <span class="k {c.kind}">{VERB[c.kind]}</span> <span class="lbl" title={c.path}>{c.label}</span>
                        <button class="link" disabled={busy} onclick={() => undo(b, c)} title="Put this back the way it was before this backup">{UNDO[c.kind]}</button>
                      </li>
                      {#if c.details?.length}<li class="det"><ul>{#each c.details as d, i (i)}<li>{d}</li>{/each}</ul></li>{/if}
                    {/each}
                  </ul>
                {/each}
              {:else}
                <p class="muted small">The starting point: {b.files} files ({size(b.bytes)}).</p>
              {/if}
              <div class="row">
                <div class="spacer"></div>
                <button class="ghost" disabled={busy} onclick={() => (confirm = b)}>Restore the pack to this point</button>
              </div>
            </div>
          {/if}
        </div>
      {:else}
        <p class="muted small empty">{tab === "manual" ? "No checkpoints yet. Make one before big changes." : "Automatic backups happen at noon and midnight, on days the pack changed."}</p>
      {/each}
    </div>

    {#if confirm}
      <div class="confirm">
        <p>Restore <strong>{inst.name}</strong> to {when(confirm.atMs)}? Mods, configs, scripts and packs go back to how they were. Worlds aren't touched. A safety backup of the current state is taken first, so you can undo this.</p>
        <div class="row"><div class="spacer"></div>
          <button class="ghost" onclick={() => (confirm = null)}>Cancel</button>
          <button class="primary" disabled={busy} onclick={() => restore(confirm!)}>{busy ? "Restoring…" : "Restore"}</button>
        </div>
      </div>
    {/if}

    <section class="opts">
      <label class="check"><input type="checkbox" bind:checked={settings.backups.auto} onchange={saveSettings} /><span>Automatic backups at noon and midnight (only when the pack changed)</span></label>
      <label class="row">Keep the newest <input class="num" type="number" min="1" max="200" bind:value={settings.backups.keepAuto} onchange={saveSettings} /> automatic backups
        and <input class="num" type="number" min="1" max="500" bind:value={settings.backups.keepManual} onchange={saveSettings} /> checkpoints</label>
    </section>
  {/if}
</div>

<style>
  .backups { display: flex; flex-direction: column; gap: 12px; min-height: 0; }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  /* A checkbox with its text beside it: long text wraps under itself, never under the box. */
  .check { display: flex; align-items: flex-start; gap: 10px; }
  .check input { flex: none; margin-top: 2px; }
  .spacer { flex: 1; }
  .muted { color: var(--muted); margin: 0; }
  .small { font-size: 12.5px; }
  .now { display: flex; flex-direction: column; gap: 8px; }
  .now .search { width: 200px; }
  .tabs { display: flex; gap: 6px; align-items: center; border-bottom: 1px solid var(--line); }
  .tabs button { background: none; border: 0; border-bottom: 2px solid transparent; padding: 6px 10px; color: var(--muted); margin-bottom: -1px; }
  .tabs button.on { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .list { display: flex; flex-direction: column; gap: 4px; max-height: 42vh; overflow-y: auto; }
  .b { border: 1px solid var(--line); border-radius: 6px; background: var(--panel); }
  .head { width: 100%; display: flex; align-items: center; gap: 10px; background: none; border: 0; padding: 8px 10px; text-align: left; }
  .caret { width: 12px; color: var(--muted); }
  .when { font-weight: 600; min-width: 170px; }
  .what { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sum { flex: none; }
  .body { padding: 4px 14px 12px 32px; display: flex; flex-direction: column; gap: 6px; }
  h5 { margin: 6px 0 0; font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); }
  .changes { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 3px; font-size: 13px; }
  .changes li { display: flex; align-items: center; gap: 8px; }
  .det ul { margin: 0 0 2px 0; padding-left: 64px; font-size: 12px; color: var(--muted); list-style: none; font-family: var(--mono, monospace); }
  .lbl { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .k { font-size: 11px; font-weight: 700; padding: 1px 6px; border-radius: 3px; min-width: 64px; text-align: center; }
  .k.added { background: color-mix(in srgb, var(--mr) 18%, transparent); color: var(--mr); }
  .k.removed { background: color-mix(in srgb, var(--danger) 18%, transparent); color: var(--danger); }
  .k.changed { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .link { background: none; border: 0; color: var(--accent); padding: 0 4px; font-size: 12.5px; }
  .confirm { border: 1px solid var(--accent); border-radius: 8px; padding: 12px 14px; display: flex; flex-direction: column; gap: 10px; background: var(--panel); }
  .confirm p { margin: 0; }
  .opts { border-top: 1px solid var(--line); padding-top: 10px; display: flex; flex-direction: column; gap: 8px; }
  .num { width: 64px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 5px; padding: 4px 6px; font: inherit; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 7px 16px; font-weight: 600; }
  .primary:disabled { opacity: 0.6; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; }
  .empty { padding: 10px; }
</style>
