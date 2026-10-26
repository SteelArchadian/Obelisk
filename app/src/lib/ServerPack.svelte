<script lang="ts">
  // Turn a profile into a server pack (only what a dedicated server needs, with the loader and
  // start scripts) and a client pack. Every mod's side is detected, shown with the evidence, and
  // can be changed before building.
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { downloadDir } from "@tauri-apps/api/path";
  import { app, toast } from "./state.svelte";
  import { cached } from "./updates.svelte";
  import type { Instance } from "./api";

  type Side = "both" | "client" | "server";
  type ModSide = { fileName: string; name: string; ids: string[]; side: Side; confidence: "sure" | "likely" | "guess"; reasons: string[]; neededBy: string[] };
  type Built = { serverDir: string; serverZip: string; clientZip: string; notes: string[] };

  let { inst }: { inst: Instance } = $props();

  let mods = $state<ModSide[] | null>(null);
  let picked = $state<Record<string, Side>>({});
  let view = $state<"all" | "client" | "check" | "server">("all");
  let query = $state("");
  let outDir = $state("");
  let memMb = $state(6144);
  let makeClient = $state(true);
  let busy = $state(false);
  let built = $state<Built | null>(null);

  const sideOf = (m: ModSide) => picked[m.fileName] ?? m.side;
  const needsCheck = (m: ModSide) => m.confidence === "guess" && sideOf(m) === m.side;

  $effect(() => {
    const platform: Record<string, string> = {};
    for (const f of cached(inst.id)?.infos ?? []) if (f.kind === "mods" && f.env) platform[f.fileName] = f.env;
    invoke<ModSide[]>("mod_sides", { instanceId: inst.id, platform })
      .then((m) => (mods = m))
      .catch((e) => { toast(String(e)); close(); });
    invoke<{ maxMemMb: number }>("profile_options", { instanceId: inst.id }).then((o) => (memMb = Math.max(4096, o.maxMemMb))).catch(() => {});
    downloadDir().then((d) => (outDir ||= d)).catch(() => {});
  });

  const close = () => (app.serverPack = null);

  const counts = $derived.by(() => {
    const c = { server: 0, client: 0, serverOnly: 0, check: 0 };
    for (const m of mods ?? []) {
      const s = sideOf(m);
      if (s !== "client") c.server++;
      if (s === "client") c.client++;
      if (s === "server") c.serverOnly++;
      if (needsCheck(m)) c.check++;
    }
    return c;
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return (mods ?? []).filter((m) => {
      if (q && !m.name.toLowerCase().includes(q) && !m.fileName.toLowerCase().includes(q)) return false;
      if (view === "client") return sideOf(m) === "client";
      if (view === "server") return sideOf(m) === "server";
      if (view === "check") return needsCheck(m);
      return true;
    });
  });

  async function pickDir() {
    const d = await open({ directory: true, title: "Where should the packs go?", defaultPath: outDir || undefined });
    if (typeof d === "string") outDir = d;
  }

  async function build() {
    if (!mods || !outDir) return;
    busy = true;
    try {
      built = await invoke<Built>("build_packs", {
        instanceId: inst.id,
        plan: {
          outDir, name: inst.name, minecraft: inst.minecraft, loader: inst.loader, loaderVersion: inst.loaderVersion,
          maxMemMb: memMb, makeClient,
          serverMods: mods.filter((m) => sideOf(m) !== "client").map((m) => m.fileName),
          clientDrop: mods.filter((m) => sideOf(m) === "server").map((m) => m.fileName),
        },
      });
    } catch (e) {
      toast(String(e));
    }
    busy = false;
  }

  const reveal = (path: string) => invoke("reveal_pack", { path }).catch((e) => toast(String(e)));
</script>

<div class="back" role="presentation" onclick={() => !busy && close()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && !busy && close()}>
    <button class="x" onclick={close} disabled={busy} aria-label="Close">✕</button>
    <h3>Server &amp; client packs · {inst.name}</h3>

    {#if built}
      <p>Done. The server pack is ready to upload, with NeoForge's installer and start scripts. Read its README for the two-step first start.</p>
      <div class="paths">
        <div><strong>Server folder</strong><button class="link" onclick={() => reveal(built!.serverDir)}>{built.serverDir}</button></div>
        <div><strong>Server zip</strong><button class="link" onclick={() => reveal(built!.serverZip)}>{built.serverZip}</button></div>
        {#if built.clientZip}<div><strong>Client pack</strong><button class="link" onclick={() => reveal(built!.clientZip)}>{built.clientZip}</button><small>Import it in Prism with Add Instance → Import.</small></div>{/if}
      </div>
      {#each built.notes as n (n)}<p class="warn">{n}</p>{/each}
      <div class="actions"><div class="spacer"></div><button class="primary" onclick={close}>Done</button></div>
    {:else if !mods}
      <p class="muted">Checking every mod: what it declares, its code, and what the download sites say…</p>
    {:else}
      <p class="muted">
        <strong class="good">{counts.server}</strong> mods go on the server ·
        <strong>{counts.client}</strong> are client-only and left out{#if counts.serverOnly} · <strong>{counts.serverOnly}</strong> server-only (left out of the client pack){/if}.
        {#if counts.check}<strong class="warn-text">{counts.check}</strong> are best guesses worth a look.{/if}
        Change any mod's side below.
      </p>

      <div class="bar">
        {#each [["all", `All (${mods.length})`], ["client", `Client only (${counts.client})`], ["check", `To check (${counts.check})`], ["server", `Server only (${counts.serverOnly})`]] as [id, label] (id)}
          <button class="chip" class:on={view === id} onclick={() => (view = id as typeof view)}>{label}</button>
        {/each}
        <input class="search" placeholder="Find a mod…" bind:value={query} />
      </div>

      <div class="list">
        {#each shown as m (m.fileName)}
          <div class="mod" class:changed={picked[m.fileName] && picked[m.fileName] !== m.side}>
            <div class="info">
              <strong>{m.name}</strong> <span class="conf {m.confidence}">{m.confidence}</span>
              <small>{m.reasons[0] ?? ""}</small>
            </div>
            <select class="select" value={sideOf(m)} onchange={(e) => (picked = { ...picked, [m.fileName]: e.currentTarget.value as Side })} aria-label="Side for {m.name}">
              <option value="both">Server + client</option>
              <option value="client">Client only</option>
              <option value="server">Server only</option>
            </select>
          </div>
        {:else}
          <p class="muted">Nothing here.</p>
        {/each}
      </div>

      <section>
        <div class="row"><span>Save to</span><button class="ghost path" onclick={pickDir} title={outDir}>{outDir || "Choose a folder…"}</button></div>
        <div class="row"><span>Server memory</span><input class="num" type="number" min="1024" step="512" bind:value={memMb} /> MB</div>
        <label class="row"><input type="checkbox" bind:checked={makeClient} /> Also make the client pack (a Prism import zip, without saves or logs)</label>
      </section>

      <div class="actions">
        <span class="muted small">Nothing in the profile is changed.</span>
        <div class="spacer"></div>
        <button class="ghost" onclick={close} disabled={busy}>Cancel</button>
        <button class="primary" onclick={build} disabled={busy || !outDir}>{busy ? "Building… (copying mods, downloading the loader)" : "Build packs"}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(820px, 94vw); max-height: 90vh; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 12px; outline: none; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  p { margin: 0; }
  .muted { color: var(--muted); }
  .good { color: var(--mr); }
  .warn-text { color: #ffb02e; }
  .warn { background: var(--err-bg); color: var(--err-text); padding: 8px 10px; border-radius: 6px; font-size: 13px; }
  .bar { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .chip { background: var(--panel); border: 1px solid var(--line); border-radius: 999px; padding: 5px 12px; font-size: 13px; }
  .chip.on { border-color: var(--accent); color: var(--accent); }
  .bar .search { margin-left: auto; width: 200px; }
  .list { flex: 1; min-height: 160px; overflow-y: auto; display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; background: var(--bg); }
  .mod { display: flex; align-items: center; gap: 12px; padding: 8px 12px; border-bottom: 1px solid var(--row-line); }
  .mod.changed { background: color-mix(in srgb, var(--accent) 10%, transparent); }
  .info { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 8px; }
  .info small { flex-basis: 100%; color: var(--muted); font-size: 12px; }
  .conf { font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.05em; padding: 1px 6px; border-radius: 3px; border: 1px solid var(--line); color: var(--muted); }
  .conf.sure { color: var(--mr); border-color: color-mix(in srgb, var(--mr) 40%, transparent); }
  .conf.guess { color: #ffb02e; border-color: rgb(255 176 46 / 0.4); }
  .select { flex: none; }
  section { display: flex; flex-direction: column; gap: 8px; border-top: 1px solid var(--line); padding-top: 12px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .row > span { width: 120px; color: var(--muted); }
  .path { flex: 1; text-align: left; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .num { width: 96px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 5px; padding: 5px 6px; font: inherit; }
  .actions { display: flex; gap: 10px; align-items: center; border-top: 1px solid var(--line); padding-top: 12px; }
  .spacer { flex: 1; }
  .small { font-size: 12.5px; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 8px 20px; font-weight: 600; }
  .primary:disabled { opacity: 0.6; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 8px 14px; }
  .paths { display: flex; flex-direction: column; gap: 8px; }
  .paths div { display: flex; flex-direction: column; gap: 2px; }
  .paths small { color: var(--muted); }
  .link { background: none; border: 0; padding: 0; color: var(--accent); text-align: left; word-break: break-all; }
</style>
