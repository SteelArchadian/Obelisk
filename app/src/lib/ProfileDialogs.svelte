<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import { app, toast } from "./state.svelte";
  import { syncInstances } from "./install";
  import Resolution from "./Resolution.svelte";
  import Backups from "./Backups.svelte";
  import { settings } from "./settings.svelte";

  type Options = {
    name: string; overrideMemory: boolean; maxMemMb: number; minMemMb: number;
    overrideJavaArgs: boolean; jvmArgs: string; overrideWindow: boolean; width: number; height: number; maximized: boolean;
    overrideJavaLocation: boolean; javaPath: string;
  };
  type Java = { path: string; version: string; major: number; vendor: string };
  let javas = $state<Java[] | null>(null);
  let need = $derived(javaFor(app.profileDialog?.inst.minecraft ?? ""));
  /** Java a Minecraft version needs (same rule as the server packs). */
  function javaFor(mc: string) {
    const [, minor = 0, patch = 0] = mc.split(".").map((n) => parseInt(n) || 0);
    return minor > 20 || (minor === 20 && patch >= 5) ? 21 : minor >= 18 ? 17 : minor === 17 ? 16 : 8;
  }
  /**
   * The Java to recommend. Minecraft 1.20.5+ runs on 25 (newest long-term release): in A/B tests on a
   * big NeoForge 1.21.1 pack it gave ~9% more FPS, half the worst stutters and faster starts. Older
   * versions stay on what they were built for, since older Forge and mixins break on newer Java.
   */
  let rec = $derived(need === 21 ? 25 : need === 16 ? 17 : need);
  /** Javas Obelisk can install for this profile: the recommended one, and the minimum if different. */
  let offers = $derived([...new Set([rec, need === 16 ? 17 : need])].filter((m) => !javas?.some((j) => j.major === m)));
  let installing = $state(0);
  let got = $state("");
  async function getJava(major: number) {
    installing = major;
    got = "";
    const stop = await listen<{ major: number; done: number; total: number }>("java-progress", (e) => {
      const mb = (n: number) => Math.round(n / 1048576);
      if (e.payload.major === major) got = e.payload.total ? `${mb(e.payload.done)} of ${mb(e.payload.total)} MB` : `${mb(e.payload.done)} MB`;
    });
    try {
      const j = await invoke<Java>("java_download", { major });
      if (javas && !javas.some((x) => x.path === j.path)) javas = [...javas, j].sort((a, b) => b.major - a.major);
      if (opts) { opts.overrideJavaLocation = true; opts.javaPath = j.path; }
      toast(`Java ${major} installed and picked for this profile. Save to keep it.`);
    } catch (e) {
      toast(String(e));
    } finally {
      stop();
      installing = 0;
    }
  }
  /** Prism writes C:/x/javaw.exe, the scan finds C:\x\javaw.exe: the same file. */
  const samePath = (a: string, b: string) => a.replace(/\\/g, "/").toLowerCase() === b.replace(/\\/g, "/").toLowerCase();
  /** A Java outside the usual folders (the profile's own, or one picked with Other Java): add it to the list. */
  async function addJava(path: string, select: boolean) {
    const j = javas?.find((x) => samePath(x.path, path)) ?? await invoke<Java | null>("java_at", { path }).catch(() => null);
    if (!j) { if (select) toast("That isn't a Java install (pick its bin\\javaw.exe)."); return; }
    if (javas && !javas.some((x) => x.path === j.path)) javas = [...javas, j].sort((a, b) => b.major - a.major);
    if (opts && (select || samePath(opts.javaPath, j.path))) opts.javaPath = j.path;
  }
  async function pickJava() {
    const f = await open({ title: "Pick java.exe or javaw.exe (in the Java folder's bin)", filters: [{ name: "Java", extensions: ["exe"] }] });
    if (typeof f === "string") await addJava(f, true);
  }
  function javaNote(j: Java | undefined, need: number) {
    if (!j) return "That Java is no longer on this PC; Prism will fail to launch until you pick another.";
    if (j.major < need) return `Too old: Minecraft needs Java ${need} or newer.`;
    if (need === 8 && j.major > 8) return "Older Minecraft and Forge often crash on Java newer than 8.";
    if (j.major === rec) return rec === need ? "The Java this Minecraft version is made for." : `Recommended: the fastest Java for this Minecraft version.`;
    if (j.major === need) return `Works. Java ${rec} runs this Minecraft version faster.`;
    return need === 17 && j.major > 17 ? "Newer than this Minecraft was built for; older Forge mods may crash." : "Newer than needed; usually fine.";
  }

  let opts = $state<Options | null>(null);
  // Memory moves in whole GB, from 2 GB up to a ceiling: 16 GB (past that, Java's garbage collector
  // pauses get longer and no Minecraft pack benefits, even on huge machines), and never so much that
  // Windows is left with under 4 GB.
  const memTop = (ramMb: number) => Math.max(2, Math.min(16, Math.floor(ramMb / 1024) - 4));
  const memAdvice = (gb: number, ramMb: number) =>
    gb <= 3 ? "Enough for vanilla and light packs."
    : gb <= 5 ? "Good for small and medium packs."
    : gb <= 8 ? "Good for big modpacks (most need 6–8 GB)."
    : gb <= 12 ? "Only huge packs or shaders with long render distances need this much."
    : gb >= Math.floor(ramMb / 1024) - 4 ? "That leaves little memory for Windows; expect stutters elsewhere."
    : "More than almost any pack uses; extra memory past this point just makes cleanup pauses longer.";
  type WinMode = "windowed" | "borderless" | "fullscreen";
  let win = $state<{ mode: WinMode; via: string | null } | null>(null);
  async function setWin(mode: WinMode) {
    const id = app.profileDialog!.inst.id;
    try {
      await invoke("set_window_mode", { instanceId: id, mode });
      win = await invoke("window_mode", { instanceId: id });
    } catch (e) { toast(String(e)); }
  }
  let ram = $state(16384);
  let prismOpen = $state(false);
  let cloneName = $state("");
  // ---- clean configs ----
  let orphans = $state<string[] | null>(null);
  let pick = $state<Set<string>>(new Set());
  let scanning = $state(false);

  async function scanConfigs(id: string) {
    scanning = true;
    try {
      orphans = await invoke<string[]>("orphan_configs", { instanceId: id });
      pick = new Set(orphans);
    } catch (e) {
      toast(String(e));
    }
    scanning = false;
  }

  async function cleanConfigs(id: string) {
    busy = true;
    try {
      const n = await invoke<number>("clean_configs", { instanceId: id, names: [...pick] });
      toast(`Removed ${n} leftover config${n === 1 ? "" : "s"}.`);
      orphans = null;
    } catch (e) {
      toast(String(e));
    }
    busy = false;
  }
  let busy = $state(false);

  $effect(() => {
    const d = app.profileDialog;
    opts = null;
    orphans = null;
    if (!d) return;
    invoke<boolean>("prism_running").then((r) => (prismOpen = r));
    if (d.kind === "options") {
      invoke<number>("system_memory_mb").then((m) => (ram = m || 16384));
      Promise.all([
        invoke<Java[]>("java_installs").then((j) => (javas = j)).catch(() => (javas = [])),
        invoke<Options>("profile_options", { instanceId: d.inst.id }).then((o) => (opts = o)).catch((e) => toast(String(e))),
      ]).then(() => { if (opts?.javaPath) addJava(opts.javaPath, false); });
      invoke<typeof win>("window_mode", { instanceId: d.inst.id }).then((w) => (win = w)).catch(() => {});
    }
    if (d.kind === "clone") cloneName = `${d.inst.name} copy`;
  });

  const close = () => (app.profileDialog = null);

  async function run(fn: () => Promise<unknown>, done: string) {
    busy = true;
    try {
      await fn();
      toast(done);
      await syncInstances();
      close();
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  const saveOptions = () => run(() => invoke("set_profile_options", { instanceId: app.profileDialog!.inst.id, options: $state.snapshot(opts) }), "Profile options saved.");
  const doClone = () => run(() => invoke("clone_instance", { instanceId: app.profileDialog!.inst.id, name: cloneName }), `Cloned as “${cloneName}”.`);
  async function doDelete() {
    const inst = app.profileDialog!.inst;
    await run(() => invoke("delete_instance", { instanceId: inst.id }), `${inst.name} moved to the modpack trash. Restore it from My Modpacks within ${Math.round(settings.packTrashHours / 24)} days.`);
    if (app.current?.id === inst.id) app.page = "library";
  }
</script>

{#if app.profileDialog}
  {@const d = app.profileDialog}
  <div class="back" role="presentation" onclick={close}>
    <div class="dialog" class:wide={d.kind === "options"} role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && close()}>
      <button class="x" onclick={close} aria-label="Close">✕</button>
      {#if prismOpen && d.kind !== "clone"}<p class="warn">Prism is open. Close it first, or it may overwrite these changes when it exits.</p>{/if}

      {#if d.kind === "options"}
        <h3>Profile settings · {d.inst.name}</h3>
        <div class="ptabs">
          <button class:on={d.tab !== "backups"} onclick={() => (app.profileDialog = { ...d, tab: "options" })}>Options</button>
          <button class:on={d.tab === "backups"} onclick={() => (app.profileDialog = { ...d, tab: "backups" })}>Backups</button>
        </div>
        {#if d.tab === "backups"}
          <Backups inst={d.inst} />
        {:else if opts}
          <label class="field">Name <input class="search wide" bind:value={opts.name} /></label>

          <section>
            <label class="row"><input type="checkbox" bind:checked={opts.overrideMemory} /> <strong>Custom memory</strong> <span class="muted">(otherwise Prism's default)</span></label>
            {#if opts.overrideMemory}
              {@const top = memTop(ram)}
              <div class="mem gb">
                <input type="range" min="2" max={top} step="1" list="mem-notches" value={Math.min(top, Math.max(2, Math.round(opts.maxMemMb / 1024)))}
                  oninput={(e) => { opts!.maxMemMb = +e.currentTarget.value * 1024; if (opts!.minMemMb > opts!.maxMemMb) opts!.minMemMb = opts!.maxMemMb; }} aria-label="Maximum memory in GB" />
                <datalist id="mem-notches">{#each Array.from({ length: top - 1 }, (_, i) => i + 2) as g (g)}<option value={g}></option>{/each}</datalist>
                <b class="gbv">{Math.round(opts.maxMemMb / 1024)} GB</b>
              </div>
              <div class="ticks" style:--n={top - 1}>{#each Array.from({ length: top - 1 }, (_, i) => i + 2) as g (g)}<span class:on={g === Math.round(opts.maxMemMb / 1024)}>{g}</span>{/each}</div>
              <p class="muted small">{memAdvice(Math.round(opts.maxMemMb / 1024), ram)} Your PC has {(ram / 1024).toFixed(0)} GB.</p>
            {/if}
          </section>

          <section>
            <label class="row"><input type="checkbox" bind:checked={opts.overrideJavaArgs} /> <strong>Custom JVM arguments</strong></label>
            {#if opts.overrideJavaArgs}<textarea class="args" bind:value={opts.jvmArgs} spellcheck="false"></textarea>{/if}
          </section>

          <section>
            <label class="row"><input type="checkbox" bind:checked={opts.overrideJavaLocation}
              onchange={() => { if (opts!.overrideJavaLocation && !opts!.javaPath && javas?.length) opts!.javaPath = (javas.find((j) => j.major === need) ?? javas[0]).path; }} />
              <strong>Choose Java</strong> <span class="muted">(otherwise Prism picks one for Minecraft {d.inst.minecraft})</span></label>
            {#if opts.overrideJavaLocation}
              {#if javas === null}<p class="muted small">Looking for Java…</p>
              {:else if javas.length === 0}<p class="muted small">No Java found on this PC. Prism downloads one by itself when this is off. <button class="link" onclick={pickJava}>Other Java…</button></p>
              {:else}
                {@const cur = javas.find((j) => samePath(j.path, opts!.javaPath))}
                <select class="select wide" bind:value={opts.javaPath} aria-label="Java for this profile">
                  {#if !cur && opts.javaPath}<option value={opts.javaPath}>{opts.javaPath} (missing)</option>{/if}
                  {#each javas as j (j.path)}
                    <option value={j.path}>Java {j.major}{j.vendor ? ` · ${j.vendor}` : ""} · {j.version}{j.major === rec ? " (recommended)" : ""}</option>
                  {/each}
                </select>
                <p class="muted small" class:warn={!cur || cur.major < need}>{javaNote(cur, need)} <button class="link" onclick={pickJava}>Other Java…</button></p>
                {#if cur && cur.major >= 24 && opts.jvmArgs.includes("ZGenerational")}<p class="muted small warn">Java {cur.major} no longer has -XX:+ZGenerational: remove it from the JVM arguments (generational ZGC is already the default).</p>{/if}
              {/if}
            {/if}
            {#if offers.length}
              <p class="muted small getjava">
                {#each offers as m (m)}
                  <button class="ghost" disabled={!!installing} onclick={() => getJava(m)}>
                    {installing === m ? `Downloading Java ${m}… ${got}` : `Install Java ${m}${m === rec ? " (recommended)" : ""}`}
                  </button>
                {/each}
                Free Eclipse Temurin from adoptium.net, about 50 MB.
              </p>
            {/if}
          </section>

          {#if win}
            <section>
              <strong>Window mode</strong>
              <div class="seg">
                {#each [["windowed", "Windowed"], ["borderless", "Borderless"], ["fullscreen", "Fullscreen"]] as [m, l] (m)}
                  <button class:on={win.mode === m} onclick={() => setWin(m as WinMode)}>{l}</button>
                {/each}
              </div>
              <p class="muted small">{#if win.mode === "borderless"}{win.via ? `Uses ${win.via} from this pack, so F11 and its in-game setting still work.` : "Obelisk makes the window borderless when the game opens (this pack has no borderless mod)."}{:else if win.mode === "fullscreen"}Exclusive fullscreen. Alt-tabbing is slower than borderless.{:else}A normal window. Size it below.{/if} Applies next time you play.</p>
            </section>
          {/if}

          <section>
            <label class="row"><input type="checkbox" bind:checked={opts.overrideWindow} /> <strong>Custom game window</strong></label>
            {#if opts.overrideWindow}
              <div class="mem"><Resolution bind:width={opts.width} bind:height={opts.height} bind:maximized={opts.maximized} /></div>
            {/if}
          </section>


          <div class="actions">
            <button class="ghost" onclick={() => invoke("reveal", { instanceId: d.inst.id, sub: "" })}>Open folder</button>
            <div class="spacer"></div>
            <button class="ghost" onclick={close}>Cancel</button>
            <button class="primary" disabled={busy} onclick={saveOptions}>Save</button>
          </div>
        {:else}<p class="muted">Loading…</p>{/if}

      {:else if d.kind === "purge"}
        <h3>Purge configs · {d.inst.name}</h3>
        {#if orphans === null}
          <p>This matches every file and folder in <code>config/</code> against the mods installed in this profile (enabled or disabled), and deletes the ones that belong to no mod.</p>
          <p class="warn">Warning: this is irreversible from Obelisk. You'll see the list before anything is deleted, and deleted files go to the Windows Recycle Bin.</p>
          <div class="actions"><div class="spacer"></div>
            <button class="ghost" onclick={close}>Cancel</button>
            <button class="primary" disabled={scanning} onclick={() => scanConfigs(d.inst.id)}>{scanning ? "Checking every mod…" : "Find leftover configs"}</button>
          </div>
        {:else if orphans.length === 0}
          <p class="muted">Nothing to purge. Every config belongs to an installed mod.</p>
          <div class="actions"><div class="spacer"></div><button class="primary" onclick={close}>Done</button></div>
        {:else}
          <p class="muted small">{orphans.length} entr{orphans.length === 1 ? "y" : "ies"} in config/ don't match any installed mod. Untick anything you want to keep.</p>
          <div class="orphans">
            {#each orphans as o (o)}
              <label class="row"><input type="checkbox" checked={pick.has(o)} onchange={() => { const n = new Set(pick); n.has(o) ? n.delete(o) : n.add(o); pick = n; }} /> {o}</label>
            {/each}
          </div>
          <div class="actions"><div class="spacer"></div>
            <button class="ghost" onclick={close}>Cancel</button>
            <button class="primary danger" disabled={busy || !pick.size} onclick={async () => { await cleanConfigs(d.inst.id); close(); }}>Delete {pick.size}</button>
          </div>
        {/if}

      {:else if d.kind === "clone"}
        <h3>Clone {d.inst.name}</h3>
        <p class="muted">Copies the whole profile, worlds included, so it can take a while for big packs. Handy for testing changes safely.</p>
        <label class="field">New name <input class="search wide" bind:value={cloneName} /></label>
        <div class="actions"><div class="spacer"></div>
          <button class="ghost" onclick={close}>Cancel</button>
          <button class="primary" disabled={busy || !cloneName.trim()} onclick={doClone}>{busy ? "Copying…" : "Clone"}</button>
        </div>

      {:else}
        <h3>Delete {d.inst.name}?</h3>
        <p class="muted">The whole profile, including its worlds, goes to the modpack trash for {Math.round(settings.packTrashHours / 24)} days. Restore it from the trash button on My Modpacks.</p>
        <div class="actions"><div class="spacer"></div>
          <button class="ghost" onclick={close}>Cancel</button>
          <button class="danger" disabled={busy} onclick={doDelete}>{busy ? "Deleting…" : "Delete profile"}</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(460px, 92vw); max-height: 88vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 14px; outline: none; }
  .dialog.wide { width: min(760px, 94vw); }
  .ptabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); }
  .ptabs button { background: none; border: 0; border-bottom: 2px solid transparent; padding: 6px 12px; color: var(--muted); margin-bottom: -1px; }
  .ptabs button.on { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  .warn { margin: 0; background: var(--err-bg); color: var(--err-text); padding: 8px 10px; border-radius: 6px; font-size: 13px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-weight: 600; }
  .wide { width: 100%; }
  section { display: flex; flex-direction: column; gap: 8px; border-top: 1px solid var(--line); padding-top: 12px; }
  .row { display: flex; align-items: center; gap: 8px; }
  .mem { display: flex; align-items: center; gap: 10px; padding-left: 24px; }
  .mem.gb input[type="range"] { flex: 1; }
  .gbv { min-width: 52px; text-align: right; }
  .ticks { display: grid; grid-template-columns: repeat(var(--n), 1fr); margin: -4px 62px 0 24px; font-size: 10.5px; color: var(--muted); text-align: center; }
  .ticks span.on { color: var(--accent); font-weight: 700; }
  .mem input[type="range"] { flex: 1; accent-color: var(--accent); }
  .args { margin-left: 24px; min-height: 70px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px; font: 12.5px "Cascadia Mono", Consolas, monospace; resize: vertical; }
  .muted { color: var(--muted); margin: 0; }
  .small { font-size: 12.5px; padding-left: 24px; }
  .actions { display: flex; gap: 10px; align-items: center; border-top: 1px solid var(--line); padding-top: 12px; }
  .spacer { flex: 1; }
  .primary, .danger { color: #fff; border: 0; border-radius: 4px; padding: 8px 20px; font-weight: 600; }
  .primary { background: var(--accent); }
  .danger { background: var(--danger); }
  .primary:disabled, .danger:disabled { opacity: 0.5; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 8px 14px; }
  .orphans { max-height: 180px; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; padding: 6px 8px; background: var(--bg); border: 1px solid var(--line); border-radius: 6px; font-size: 13px; }
  .primary.danger { background: var(--danger); }
  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 6px; overflow: hidden; width: fit-content; }
  .seg button { background: none; border: 0; padding: 6px 16px; color: var(--text-2); }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button.on { background: var(--accent); color: #fff; }
  .link { background: none; border: 0; color: var(--accent); padding: 0 2px; cursor: pointer; }
  .getjava { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-top: 8px; }
</style>
