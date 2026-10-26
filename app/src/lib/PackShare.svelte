<script lang="ts">
  // Share → two ways out: a 6-character code a friend pastes into Import (good for 30 minutes,
  // served straight from this Obelisk), or the whole pack zipped into Downloads.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { app, toast } from "./state.svelte";
  import type { Instance } from "./api";

  let { inst }: { inst: Instance } = $props();
  let busy = $state<"" | "code" | "zip">("");
  let zipDone = $state(0);
  let zipTotal = $state(0);
  let zipPath = $state("");
  let now = $state(Date.now());

  const live = $derived.by(() => {
    const c = app.packCodes[inst.id];
    return c && c.expiresMs > now ? c : null;
  });
  const left = $derived(live ? Math.max(0, live.expiresMs - now) : 0);
  const clock = $derived(`${Math.floor(left / 60000)}:${String(Math.floor((left % 60000) / 1000)).padStart(2, "0")}`);

  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

  const close = () => (app.packShare = null);

  async function getCode() {
    busy = "code";
    try {
      app.packCodes[inst.id] = await invoke("pack_code_start", { instanceId: inst.id });
      now = Date.now();
    } catch (e) { toast(String(e)); } finally { busy = ""; }
  }

  async function stopCode() {
    if (!live) return;
    try { await invoke("pack_code_stop", { code: live.code }); } catch (e) { toast(String(e)); }
    delete app.packCodes[inst.id];
  }

  const copy = () => live && navigator.clipboard.writeText(live.code).then(() => toast("Code copied. Your friend pastes it into Import."));

  async function exportZip() {
    busy = "zip";
    zipDone = zipTotal = 0;
    zipPath = "";
    const un = await listen<{ done: number; total: number }>("pack-export", (e) => { zipDone = e.payload.done; zipTotal = e.payload.total; });
    try { zipPath = await invoke<string>("pack_export_zip", { instanceId: inst.id }); }
    catch (e) { toast(String(e)); }
    finally { un(); busy = ""; }
  }
</script>

<div class="back" role="presentation" onclick={() => !busy && close()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && !busy && close()}>
    <button class="x" onclick={close} disabled={!!busy} aria-label="Close">✕</button>
    <h3>Share · {inst.name}</h3>

    {#if live}
      <section class="code-box">
        <p class="muted small">Give this code to your friend. They click Import in Obelisk and paste it.</p>
        <div class="code-row">
          <span class="code">{live.code}</span>
          <button class="ghost" onclick={copy}>Copy</button>
        </div>
        <p class="small">Expires in <b>{clock}</b> · {live.files} files{#if live.custom} · {live.custom} custom{/if}</p>
        {#if live.private}<p class="muted small">{live.private} config {live.private === 1 ? "file stays" : "files stay"} on your PC: {live.private === 1 ? "it seems" : "they seem"} to hold a password, token or webhook.</p>{/if}
        <p class="muted small">Keep Obelisk open until they're done: the files come from your PC, or from Modrinth and CurseForge where those have them.</p>
        <div class="row"><span class="spacer"></span><button class="ghost" onclick={stopCode}>Stop sharing</button></div>
      </section>
    {:else if zipPath}
      <section>
        <p>Zipped into your Downloads folder:</p>
        <p class="muted small path">{zipPath}</p>
        <div class="row"><span class="spacer"></span><button class="primary" onclick={close}>Done</button></div>
      </section>
    {:else}
      <div class="choices">
        <button class="choice" disabled={!!busy} onclick={getCode}>
          <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true"><rect x="3" y="6" width="18" height="12" rx="2.5" fill="none" stroke="currentColor" stroke-width="1.8" /><path d="M7 12h.01M10.5 12h.01M14 12h.01M17.5 12h.01" stroke="currentColor" stroke-width="2.6" stroke-linecap="round" /></svg>
          <b>{busy === "code" ? "Making a code…" : "Get a code"}</b>
          <span class="muted small">A 6-character code your friend pastes into Import. Good for 30 minutes.</span>
        </button>
        <button class="choice" disabled={!!busy} onclick={exportZip}>
          <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true"><path d="M6 3h8l4 4v14H6z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" /><path d="M11 6h2M11 9h2M11 12h2" stroke="currentColor" stroke-width="1.8" /><rect x="10.5" y="14" width="3" height="4" rx="0.8" fill="none" stroke="currentColor" stroke-width="1.6" /></svg>
          <b>{busy === "zip" ? (zipTotal ? `Zipping… ${zipDone} / ${zipTotal}` : "Zipping…") : "Export as zip"}</b>
          <span class="muted small">The whole pack in one file in Downloads. Worlds and your game settings stay out.</span>
        </button>
      </div>
      {#if busy === "zip" && zipTotal}<div class="bar"><i style:width="{(zipDone / zipTotal) * 100}%"></i></div>{/if}
    {/if}
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(560px, 94vw); max-height: 90vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 14px; outline: none; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  p { margin: 0; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  section { display: flex; flex-direction: column; gap: 8px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .choices { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .choice { display: flex; flex-direction: column; align-items: flex-start; gap: 6px; text-align: left; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 16px; color: var(--text); }
  .choice:hover:not(:disabled) { border-color: var(--accent); }
  .choice:disabled { opacity: 0.7; }
  .choice svg { color: var(--accent); }
  .code-row { display: flex; align-items: center; gap: 12px; }
  .code { font: 700 34px ui-monospace, Consolas, monospace; letter-spacing: 0.18em; background: var(--panel); border: 1px solid var(--line); border-radius: 8px; padding: 8px 18px; user-select: all; }
  .path { word-break: break-all; }
  .bar { height: 6px; background: var(--panel); border-radius: 3px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 7px 16px; font-weight: 600; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; }
</style>
