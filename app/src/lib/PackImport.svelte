<script lang="ts">
  // Import → code: paste a friend's 6-character Share code and go. If the pack holds files no
  // site vouches for (custom jars, scripts), ask first: only the known files, or those too.
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { app, toast, toastError } from "./state.svelte";
  import { syncInstances } from "./install";
  import { open } from "@tauri-apps/plugin-dialog";
  import { api } from "./api";

  type Findings = { serious: string[]; notes: string[] };
  type Checked = { path: string; size: number; findings: Findings };
  type Scan = { result: "clean" } | { result: "threat" | "unavailable"; detail: string };
  type Game = { java?: number; minMemMb?: number; maxMemMb?: number; jvmArgs?: string; droppedArgs?: string[] };
  type Preview = { invite?: string | null; name: string; files: number; bytes: number; known: number; custom: Checked[]; refused: Checked[]; scan: Scan; game?: Game | null };
  type Report = { id: string; leftOut: string[]; scan: Scan };

  let code = $state(app.packImport ?? "");
  let stage = $state<"enter" | "looking" | "custom" | "installing" | "settings" | "invite">("enter");
  // After install: the sharer's own Java, memory and JVM arguments, each taken only if ticked.
  let newId = $state("");
  let take = $state({ java: true, memory: true, args: true });
  let ramMb = $state(0);
  let applying = $state("");
  let joining = $state(false);
  let preview = $state<Preview | null>(null);
  let done = $state(0);
  let total = $state(0);
  let current = $state("");
  // What the backend is doing, and when it last moved: no news for a while means something is stuck.
  let step = $state("");
  let lastMove = $state(Date.now());
  let now = $state(Date.now());
  const busy = $derived(stage === "looking" || stage === "installing");
  const quiet = $derived(busy ? Math.floor((now - lastMove) / 1000) : 0);
  const STEPS: Record<string, string> = { checking: "Checking your friend's custom files", installing: "Installing", scanning: "Scanning" };
  const status = $derived(joining ? "Downloading the pack from your friend's PC…" : !step ? "Connecting to your friend's PC…"
    : step === "scanning" ? current
    : `${STEPS[step] ?? step} · ${done} / ${total}`);

  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    const un = listen<{ step: string; done: number; total: number; file: string }>("pack-import", (e) => {
      ({ step, done, total, file: current } = e.payload);
      lastMove = Date.now();
    });
    return () => { clearInterval(t); un.then((u) => u()); };
  });
  const startWatch = () => { step = current = ""; done = total = 0; lastMove = now = Date.now(); };
  const stop = () => invoke("pack_code_cancel").catch(() => {});

  const close = () => (app.packImport = null);
  const mb = (b: number) => (b / 1048576).toFixed(b < 10485760 ? 1 : 0) + " MB";

  // A failed import or join: what went wrong, and Details (Obelisk's step-by-step readout) to look into it.
  let failed = $state("");
  let details = $state<string[] | null>(null);
  const fail = (e: unknown) => { failed = String(e); details = null; stage = "enter"; };
  const showDetails = async () => (details = details ? null : await invoke<string[]>("share_trace").catch(() => ["(no readout)"]));
  const copyDetails = () => navigator.clipboard.writeText([failed, "", ...(details ?? [])].join("\n")).then(() => toast("Details copied."));

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
    failed = "";
    startWatch();
    try {
      preview = await invoke<Preview>("pack_code_preview", { code });
      if (preview.invite) stage = "invite";
      else if (preview.custom.length || preview.refused.length || preview.scan.result !== "clean") stage = "custom";
      else install(false);
    } catch (e) { fail(e); }
  }

  async function fromFile() {
    const path = await open({ title: "Import a modpack", filters: [{ name: "Modpacks", extensions: ["zip", "mrpack"] }] });
    if (typeof path !== "string") return;
    try {
      await api.importPack(path);
      toast("Opened in Prism's importer. Press Sync when it finishes.");
      close();
    } catch (e) { toastError(e); }
  }

  async function install(withCustom: boolean) {
    stage = "installing";
    startWatch();
    try {
      const r = await invoke<Report>("pack_code_import", { code, withCustom });
      await syncInstances();
      const out = r.leftOut.length ? ` ${r.leftOut.length} ${r.leftOut.length === 1 ? "file was" : "files were"} left out.` : "";
      const scan = r.scan.result === "unavailable" ? " Windows Defender couldn't scan it; your own antivirus should." : "";
      toast(`${preview?.name ?? "The pack"} is installed. It's in your Library.${out}${scan}`);
      const g = preview?.game;
      if (g && (g.java || g.maxMemMb || g.jvmArgs)) {
        newId = r.id;
        ramMb = await invoke<number>("system_memory_mb").catch(() => 0);
        take = { java: !!g.java, memory: !!g.maxMemMb && (!ramMb || g.maxMemMb <= ramMb * 0.75), args: !!g.jvmArgs };
        stage = "settings";
      } else close();
    } catch (e) { fail(e); }
  }

  // A Collaborate code: join the pack (downloaded from the friend) and keep it in step with theirs.
  async function join() {
    if (!preview?.invite) return;
    joining = true;
    stage = "installing";
    startWatch();
    try {
      const id = await invoke<string>("share_join", { code: preview.invite });
      await syncInstances();
      const held = (await invoke<{ held?: unknown[] }>("share_status", { instanceId: id }).catch(() => null))?.held?.length ?? 0;
      toast(`You're collaborating on ${preview.name}. It's in your Library; changes come and go from Collaborate.`
        + (held ? ` Obelisk blocked ${held} file${held === 1 ? "" : "s"}: review ${held === 1 ? "it" : "them"} under Collaborate → Blocked files.` : ""));
      close();
    } catch (e) { fail(e); } finally { joining = false; }
  }

  type Options = { overrideMemory: boolean; maxMemMb: number; minMemMb: number; overrideJavaArgs: boolean; jvmArgs: string;
    overrideJavaLocation: boolean; javaPath: string } & Record<string, unknown>;

  async function useSettings() {
    const g = preview?.game;
    if (!g) return close();
    try {
      const o = await invoke<Options>("profile_options", { instanceId: newId });
      if (take.java && g.java) {
        const installs = await invoke<{ path: string; major: number }[]>("java_installs");
        let j = installs.find((i) => i.major === g.java);
        if (!j) {
          applying = `Downloading Java ${g.java}…`;
          j = await invoke<{ path: string; major: number }>("java_download", { major: g.java });
        }
        o.overrideJavaLocation = true;
        o.javaPath = j.path;
      }
      if (take.memory && g.maxMemMb) {
        o.overrideMemory = true;
        o.maxMemMb = g.maxMemMb;
        o.minMemMb = Math.min(g.minMemMb ?? o.minMemMb, g.maxMemMb);
      }
      if (take.args && g.jvmArgs) {
        o.overrideJavaArgs = true;
        o.jvmArgs = g.jvmArgs;
      }
      await invoke("set_profile_options", { instanceId: newId, options: o });
      toast("Your friend's settings are on the pack. You can change them in its Profile options.");
      close();
    } catch (e) { toastError(e); applying = ""; }
  }
  const gb = (m: number) => (m / 1024).toFixed(m % 1024 ? 1 : 0) + " GB";
</script>

<div class="back" role="presentation" onclick={() => stage !== "installing" && close()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && stage !== "installing" && close()}>
    <button class="x" onclick={close} disabled={stage === "installing"} aria-label="Close">✕</button>
    <h3>Import a pack by code</h3>

    {#if stage === "enter" || stage === "looking"}
      <p class="muted small">Paste the code from your friend's Share. Their Obelisk needs to be open.</p>
      <div class="row">
        <input type="search" class="code mono" maxlength="12" placeholder="ABC123" bind:value={code} disabled={stage === "looking"}
          onkeydown={(e) => e.key === "Enter" && go()} />
        <button class="go" disabled={stage === "looking"} onclick={pasteAndGo} title="Paste & go" aria-label="Paste and go">
          <svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><rect x="8" y="3" width="12" height="15" rx="2" fill="none" stroke="currentColor" stroke-width="2" /><path d="M5 7v12a2 2 0 0 0 2 2h9" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
          {stage === "looking" ? "Finding…" : "Paste & go"}
        </button>
      </div>
      {#if stage === "looking"}
        {@render progress()}
      {:else if failed}
        <div class="failed">
          <span>{failed}</span>
          <div class="row"><button class="link small" onclick={showDetails}>{details ? "Hide details" : "Details"}</button>
            {#if details}<button class="link small" onclick={copyDetails}>Copy</button>{/if}</div>
          {#if details}<pre class="trace">{details.join("\n")}</pre>{/if}
        </div>
        <button class="link small" onclick={fromFile}>Or import a .zip / .mrpack file…</button>
      {:else}
        <button class="link small" onclick={fromFile}>Or import a .zip / .mrpack file…</button>
      {/if}
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
    {:else if stage === "invite" && preview}
      <p>You're invited to collaborate on <b>{preview.name}</b>.</p>
      <ul class="muted small">
        <li>Obelisk downloads the pack from your friend's PC and sets it up to collaborate.</li>
        <li>When either of you adds or removes mods or changes configs, the other gets those changes to review.</li>
        <li>Worlds and personal game settings aren't shared. You can turn collaboration off any time.</li>
      </ul>
      <div class="row"><span class="spacer"></span>
        <button class="ghost" onclick={close}>Not now</button>
        <button class="primary" onclick={join}>Join</button>
      </div>
    {:else if stage === "settings" && preview?.game}
      {@const g = preview.game}
      <p>Your friend set up {preview.name} with these. Use them too?</p>
      <p class="muted small">Your choice: Obelisk's own picks work fine. You can change any of this later in the pack's Profile options.</p>
      {#if g.java}
        <label class="pick"><input type="checkbox" bind:checked={take.java} disabled={!!applying} />
          <span>Java {g.java}<br /><span class="muted small">Obelisk downloads it if you don't have it.</span></span></label>
      {/if}
      {#if g.maxMemMb}
        <label class="pick"><input type="checkbox" bind:checked={take.memory} disabled={!!applying} />
          <span>Memory: {g.minMemMb ? `${gb(g.minMemMb)} to ` : "up to "}{gb(g.maxMemMb)}
            {#if ramMb}<br /><span class="muted small" class:warnText={g.maxMemMb > ramMb * 0.75}>This PC has {gb(ramMb)}.{g.maxMemMb > ramMb * 0.75 ? " That's a lot of it; Windows may slow down." : ""}</span>{/if}</span></label>
      {/if}
      {#if g.jvmArgs}
        <label class="pick"><input type="checkbox" bind:checked={take.args} disabled={!!applying} />
          <span>Java arguments<br /><code class="small">{g.jvmArgs}</code></span></label>
      {/if}
      {#if g.droppedArgs?.length}
        <p class="muted small">Left out for safety (they could load or run something): <code>{g.droppedArgs.join(" ")}</code></p>
      {/if}
      {#if applying}<p class="small">{applying}</p><div class="bar busy"><i></i></div>{/if}
      <div class="row"><span class="spacer"></span>
        <button class="ghost" disabled={!!applying} onclick={close}>Keep mine</button>
        <button class="primary" disabled={!!applying} onclick={useSettings}>Use the ticked ones</button>
      </div>
    {:else}
      <p>Installing {preview?.name ?? "the pack"}…</p>
      {@render progress()}
    {/if}
  </div>
</div>

{#snippet progress()}
  <div class="bar" class:busy={!total || step === "scanning"}><i style:width={total && step !== "scanning" ? `${(done / total) * 100}%` : ""}></i></div>
  <p class="small">{status}</p>
  {#if current && step !== "scanning"}<p class="muted small path">{current}</p>{/if}
  {#if step === "scanning"}
    <p class="muted small">This runs on your PC only. Big files take Defender a while.</p>
  {:else if quiet >= 20 && !joining}
    <p class="stall small">Nothing has moved for {quiet} seconds. Your friend's PC may have gone to sleep or closed Obelisk; Obelisk gives up by itself after 45 seconds without data.</p>
  {/if}
  <div class="row"><span class="spacer"></span><button class="ghost" onclick={stop}>Stop</button></div>
{/snippet}

<style>
  .pick { display: flex; gap: 8px; align-items: flex-start; margin: 8px 0; }
  .pick code, code { word-break: break-all; }
  .warnText { color: var(--err-text); }
  .bar.busy i { width: 30%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(340%); } }
  .stall { color: var(--err-text); background: var(--err-bg); border-radius: 6px; padding: 8px 10px; }
  .failed { color: var(--err-text); background: var(--err-bg); border-radius: 6px; padding: 8px 10px; display: flex; flex-direction: column; gap: 6px; font-size: 13px; }
  .trace { margin: 0; max-height: 220px; overflow: auto; font-size: 11px; line-height: 1.4; color: var(--text); background: var(--panel); border-radius: 4px; padding: 6px 8px; white-space: pre-wrap; word-break: break-all; user-select: text; }
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(560px, 94vw); max-height: 90vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 14px; outline: none; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  p { margin: 0; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .code { flex: 1; font: 700 24px Consolas, ui-monospace, monospace; letter-spacing: 0.16em; text-transform: uppercase; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 8px; padding: 8px 14px; }
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
