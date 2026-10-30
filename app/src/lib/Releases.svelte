<script lang="ts">
  // Mod releases on this PC's test bench: each mod's stable and test line, which version every pack has, and the
  // rollout queue. Shown only when a test bench is set up. Chats submit builds; the user can promote, roll back and
  // opt a pack out here.
  import { invoke } from "@tauri-apps/api/core";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import { toast, toastError } from "./state.svelte";

  type Line = { version: string; kind: string; owner: string; notes: string; promoted: string } | null;
  type Status = {
    mods: { mod: string; stable: Line; test: Line; notices: string[] }[];
    packs: { pack: string; name: string; optOut: boolean; mods: Record<string, { file: string; version: string | null; line: string }> }[];
    queue: { id: number; modId: string; version: string; pack: string; state: string; detail: string }[];
    recent: { id: number; modId: string; version: string; pack: string; state: string; detail: string; updated: number }[];
  };

  let st = $state<Status | null>(null);
  let busy = $state(false);

  async function load() {
    try { st = await invoke<Status>("releases_call", { command: "release-status", args: {} }); } catch { st = null; }
  }

  async function call(command: string, args: Record<string, unknown>, done: string) {
    busy = true;
    try { await invoke("releases_call", { command, args }); toast(done); await load(); }
    catch (e) { toastError(String(e)); }
    finally { busy = false; }
  }

  async function promote(mod: string, version: string) {
    if (!(await ask(`Make ${mod} ${version} stable? It then goes into every pack that has ${mod}, each after its own safety test.`, { title: "Promote to stable", kind: "info" }))) return;
    call("release-promote", { mod, version, reason: "user-played" }, `${mod} ${version} is stable now`);
  }

  async function rollback(mod: string, pack: string, name: string) {
    if (!(await ask(`Put back the previous ${mod} in ${name}?`, { title: "Roll back", kind: "warning" }))) return;
    call("release-rollback", { mod, pack, reason: "rolled back by the user" }, `Rolled back ${mod} in ${name}`);
  }

  const installed = (mod: string, pack: string) =>
    st?.recent.some((o) => o.modId === mod && o.pack === pack && o.state === "installed") ?? false;

  onMount(() => {
    load();
    const t = setInterval(load, 30000);
    return () => clearInterval(t);
  });
</script>

{#if st && st.mods.length}
  <section class="rel">
    <h2>Mod releases</h2>
    <p class="muted small">Builds made on this PC. Fixes are stable and go into every pack that has the mod; features are test builds and go into TEST DUMMY first. Every install waits until the pack is closed, passes a 2-cycle safety test in a copy of that pack, and keeps the old jar. A crash caused by a new build puts the old one back by itself.</p>

    <table>
      <thead><tr><th>Mod</th><th>Stable</th><th>Test</th>{#each st.packs as p}<th>{p.name}</th>{/each}</tr></thead>
      <tbody>
        {#each st.mods as m}
          <tr>
            <td><strong>{m.mod}</strong></td>
            <td>{m.stable?.version ?? "–"}</td>
            <td>
              {#if m.test}
                {m.test.version}
                <button class="link" disabled={busy} onclick={() => promote(m.mod, m.test!.version)}>Make stable</button>
              {:else}–{/if}
            </td>
            {#each st.packs as p}
              {@const v = p.mods[m.mod]}
              <td>
                {#if v}
                  <span class="tag {v.line}">{v.version ?? "other"}</span>
                  {#if installed(m.mod, p.pack)}<button class="link" disabled={busy} onclick={() => rollback(m.mod, p.pack, p.name)}>Roll back</button>{/if}
                {:else}<span class="muted">–</span>{/if}
              </td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>

    {#if st.queue.length}
      <h3>In progress</h3>
      <ul>
        {#each st.queue as o}<li>{o.modId} {o.version} → {o.pack}: {o.state}{o.detail ? `, ${o.detail}` : ""}</li>{/each}
      </ul>
    {/if}
    {#if st.recent.length}
      <h3>Recent</h3>
      <ul>
        {#each st.recent.slice(0, 10) as o}<li class:bad={o.state === "failed" || o.state === "rolled-back"}>{o.modId} {o.version} → {o.pack}: {o.state}{o.detail ? `, ${o.detail}` : ""}</li>{/each}
      </ul>
    {/if}

    <h3>Packs</h3>
    {#each st.packs as p}
      <label class="row-check">
        <input type="checkbox" checked={!p.optOut} disabled={busy}
          onchange={(e) => call("release-optout", { pack: p.pack, on: !(e.currentTarget as HTMLInputElement).checked },
            (e.currentTarget as HTMLInputElement).checked ? `${p.name} takes releases again` : `${p.name} takes no automatic releases`)} />
        {p.name} gets stable releases automatically
      </label>
    {/each}
  </section>
{/if}

<style>
  .rel { margin-top: 18px; display: flex; flex-direction: column; gap: 8px; }
  h2 { font-size: 15px; margin: 0; }
  h3 { font-size: 13px; margin: 6px 0 0; }
  .muted { color: var(--muted); margin: 0; line-height: 1.5; }
  .small { font-size: 12px; }
  table { border-collapse: collapse; font-size: 12px; width: 100%; display: block; overflow-x: auto; }
  th, td { text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--line, rgba(127, 127, 127, 0.2)); white-space: nowrap; }
  .tag { padding: 1px 6px; border-radius: 4px; background: rgba(127, 127, 127, 0.15); }
  .tag.stable { background: rgba(60, 170, 90, 0.2); }
  .tag.test { background: rgba(220, 160, 40, 0.2); }
  .link { background: none; border: 0; padding: 0 0 0 6px; color: var(--accent); cursor: pointer; font-size: 12px; }
  ul { margin: 0; padding-left: 18px; font-size: 12px; }
  li.bad { color: var(--danger, #d55); }
  .row-check { display: flex; align-items: center; gap: 8px; font-size: 12px; }
</style>
