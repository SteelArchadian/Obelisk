<script lang="ts">
  // Import → code: paste a friend's 6-character Share code and go. If the pack holds files no
  // site vouches for (custom jars, scripts), ask first: only the known files, or those too.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { app, toast } from "./state.svelte";
  import { syncInstances } from "./install";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "./api";

  type Findings = { serious: string[]; notes: string[] };
  type Checked = { path: string; size: number; findings: Findings };
  type Scan = { result: "clean" } | { result: "threat" | "unavailable"; detail: string };
  type Preview = { name: string; files: number; bytes: number; known: number; custom: Checked[]; refused: Checked[]; scan: Scan };
  type Report = { id: string; leftOut: string[]; scan: Scan };

  let code = $state(app.packImport ?? "");
  let stage = $state<"enter" | "looking" | "custom" | "installing">("enter");
  let preview = $state<Preview | null>(null);
  let done = $state(0);
  let total = $state(0);
  let current = $state("");

  const close = () => (app.packImport = null);
  const mb = (b: number) => (b / 1048576).toFixed(b < 10485760 ? 1 : 0) + " MB";

  async function pasteAndGo() {
    try {
      const clip = (await navigator.clipboard.readText()).trim();
      if (clip && clip.length <= 12) code = clip;
    } catch { /* fall back to what's typed */ }
    go();
  }

  async function go() {
    if (!code.trim()) return toast("Paste the code your friend gave you.");
    stage = "looking";
    try {
      preview = await invoke<Preview>("pack_code_preview", { code });
      if (preview.custom.length || preview.refused.length || preview.scan.result !== "clean") stage = "custom";
      else install(false);
    } catch (e) { toast(String(e)); stage = "enter"; }
  }

  async function fromFile() {
    const path = await open({ title: "Import a modpack", filters: [{ name: "Modpacks", extensions: ["zip", "mrpack"] }] });
    if (typeof path !== "string") return;
    try {
      await api.importPack(path);
      toast("Opened in Prism's importer. Press Sync when it finishes.");
      close();
    } catch (e) { toast(String(e)); }
  }

  async function install(withCustom: boolean) {
    stage = "installing";
    done = total = 0;
    const un = await listen<{ done: number; total: number; file: string }>("pack-import", (e) => { ({ done, total, file: current } = e.payload); });
    try {
      const r = await invoke<Report>("pack_code_import", { code, withCustom });
      await syncInstances();
      const out = r.leftOut.length ? ` ${r.leftOut.length} ${r.leftOut.length === 1 ? "file was" : "files were"} left out.` : "";
      const scan = r.scan.result === "unavailable" ? " Windows Defender couldn't scan it; your own antivirus should." : "";
      toast(`${preview?.name ?? "The pack"} is installed. It's in your Library.${out}${scan}`);
      close();
    } catch (e) { toast(String(e)); stage = "enter"; }
    finally { un(); }
  }
</script>

<div class="back" role="presentation" onclick={() => stage !== "installing" && close()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && stage !== "installing" && close()}>
    <button class="x" onclick={close} disabled={stage === "installing"} aria-label="Close">✕</button>
    <h3>Import a pack by code</h3>

    {#if stage === "enter" || stage === "looking"}
      <p class="muted small">Paste the code from your friend's Share. Their Obelisk needs to be open.</p>
      <div class="row">
        <input class="code" maxlength="12" placeholder="ABC123" bind:value={code} disabled={stage === "looking"}
          onkeydown={(e) => e.key === "Enter" && go()} />
        <button class="go" disabled={stage === "looking"} onclick={pasteAndGo} title="Paste & go" aria-label="Paste and go">
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><rect x="8" y="3" width="12" height="15" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M5 7v12a2 2 0 0 0 2 2h9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
          {stage === "looking" ? "Finding…" : "Paste & go"}
        </button>
      </div>
      <button class="link small" disabled={stage === "looking"} onclick={fromFile}>Or import a .zip / .mrpack file…</button>
    {:else if stage === "custom" && preview}
      {#if preview.scan.result === "threat"}
        <div class="warn">
          <b>Windows Defender found a threat in {preview.name}.</b>
          <p class="small">{preview.scan.detail}</p>
          <p class="small">Obelisk won't install this pack. Let whoever sent the code know: their PC may be infected.</p>
        </div>
        <div class="row"><span class="spacer"></span><button class="ghost" onclick={close}>Close</button></div>
      {:else}
        {#if preview.refused.length}
          <div class="warn">
            <b>{preview.refused.length} {preview.refused.length === 1 ? "file" : "files"} won't be installed.</b>
            <p class="small">Obelisk never takes Windows programs or scripts from another PC, or code that shows signs of malware.</p>
            <ul class="small">
              {#each preview.refused.slice(0, 20) as f}<li class="col"><span>{f.path}</span><span class="why">{f.findings.serious.join("; ")}</span></li>{/each}
            </ul>
          </div>
        {/if}
        {#if preview.custom.length}
          <div class="warn soft">
            <b>{preview.name} has {preview.custom.length} custom {preview.custom.length === 1 ? "file" : "files"}.</b>
            <p class="small">Modrinth and CurseForge vouch for {preview.known} of its {preview.files} files. These come only from your friend's PC. Obelisk opened them and found no signs of malware{preview.scan.result === "clean" ? ", and Windows Defender found nothing" : ""}, but no site has reviewed them. Take them only if you trust who sent the code.</p>
            {#if preview.scan.result === "unavailable"}<p class="small">Windows Defender couldn't scan them ({preview.scan.detail}).</p>{/if}
            <ul class="small">
              {#each preview.custom.slice(0, 40) as f}
                <li class="col"><span class="line"><span>{f.path}</span><span class="muted">{mb(f.size)}</span></span>
                  {#if f.findings.notes.length}<span class="muted">{f.findings.notes.join("; ")}</span>{/if}</li>
              {/each}
              {#if preview.custom.length > 40}<li class="muted">…and {preview.custom.length - 40} more</li>{/if}
            </ul>
          </div>
        {/if}
        <div class="row">
          <span class="spacer"></span>
          {#if preview.custom.length}
            <button class="ghost" onclick={() => install(false)}>Only known files</button>
            <button class="primary danger" onclick={() => install(true)}>Include custom files</button>
          {:else}
            <button class="ghost" onclick={close}>Cancel</button>
            <button class="primary" onclick={() => install(false)}>Install the rest</button>
          {/if}
        </div>
      {/if}
    {:else}
      <p>Installing {preview?.name ?? "the pack"}…</p>
      <div class="bar"><i style:width="{total ? (done / total) * 100 : 0}%"></i></div>
      <p class="muted small path">{total ? `${done} / ${total}` : "Starting…"} {current}</p>
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
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .code { flex: 1; font: 700 24px ui-monospace, Consolas, monospace; letter-spacing: 0.16em; text-transform: uppercase; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 8px; padding: 8px 14px; }
  .go { display: inline-flex; align-items: center; gap: 8px; color: #fff; background: var(--accent); border: 0; border-radius: 8px; padding: 12px 16px; font-weight: 600; }
  .go:disabled { opacity: 0.7; }
  .warn { border: 1px solid var(--danger); background: color-mix(in srgb, var(--danger) 10%, transparent); border-radius: 8px; padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
  .warn b { color: var(--danger); }
  .warn.soft { border-color: color-mix(in srgb, var(--danger) 50%, var(--line)); }
  li.col { flex-direction: column; gap: 1px; }
  .line { display: flex; justify-content: space-between; gap: 12px; }
  .why { color: var(--danger); }
  ul { list-style: none; margin: 0; padding: 0; max-height: 200px; overflow-y: auto; display: flex; flex-direction: column; gap: 3px; }
  li { display: flex; justify-content: space-between; gap: 12px; }
  li span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link { align-self: flex-start; background: none; border: 0; padding: 0; color: var(--accent); }
  .path { word-break: break-all; }
  .bar { height: 6px; background: var(--panel); border-radius: 3px; overflow: hidden; }
  .bar i { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 7px 16px; font-weight: 600; }
  .primary.danger { background: var(--danger); }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; }
</style>
