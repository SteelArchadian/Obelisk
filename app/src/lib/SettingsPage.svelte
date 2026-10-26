<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import { listen } from "@tauri-apps/api/event";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { onMount } from "svelte";
  import { app, toast } from "./state.svelte";
  import { syncInstances } from "./install";
  import { resetCfSearch } from "./cf";
  import { settings, saveSettings, THEMES } from "./settings.svelte";
  import Resolution from "./Resolution.svelte";

  // About → Check for updates: the same GitHub check Obelisk runs every 30 minutes, on demand.
  type SelfUpdate = { version: string; notes: string };
  let version = $state("");
  getVersion().then((v) => (version = v)).catch(() => {});
  let updateState = $state<"" | "checking" | "current" | "installing">("");
  let found = $state<SelfUpdate | null>(null);
  async function checkUpdates() {
    updateState = "checking";
    found = null;
    try {
      found = await invoke<SelfUpdate | null>("self_update_check");
      updateState = found ? "" : "current";
    } catch (e) {
      updateState = "";
      toast(`Couldn't check for updates: ${e}`);
    }
  }
  async function installUpdate() {
    updateState = "installing";
    try {
      await invoke("self_update_install");
    } catch (e) {
      updateState = "";
      toast(String(e));
    }
  }

  type Launcher = { id: string; name: string; found: boolean; supported: boolean; location: string };
  type Source = { launcher: string; name: string; path: string; minecraft: string; loaderUid: string; loaderVersion: string; secondsPlayed: number; syncedAs: string | null };
  type Defaults = { maxMemMb: number; minMemMb: number; jvmArgs: string; width: number; height: number; maximized: boolean };

  const SECTIONS = [
    ["general", "General"], ["sync", "Sync & Import"], ["themes", "Themes"],
    ["minecraft", "Minecraft & Java"], ["curseforge", "CurseForge API"], ["plugins", "Plugins"], ["about", "About"],
  ] as const;

  let launchers = $state<Launcher[]>([]);
  let sources = $state<Source[]>([]);
  let syncing = $state<Record<string, number>>({});
  let defaults = $state<Defaults | null>(null);
  let ram = $state(16384);
  let prismOpen = $state(false);
  let hasKey = $state(false);
  let keyInput = $state("");

  const LOADER_NAMES: Record<string, string> = {
    "net.neoforged": "NeoForge", "net.minecraftforge": "Forge", "net.fabricmc.fabric-loader": "Fabric", "org.quiltmc.quilt-loader": "Quilt",
  };

  onMount(() => {
    refresh();
    invoke<Defaults>("prism_defaults").then((d) => (defaults = d));
    invoke<number>("system_memory_mb").then((m) => (ram = m || 16384));
    invoke<boolean>("prism_running").then((r) => (prismOpen = r));
    invoke<boolean>("has_cf_key").then((k) => (hasKey = k));
    const un = listen<{ path: string; bytes: number }>("sync-progress", (e) => {
      if (e.payload.path in syncing) syncing[e.payload.path] = e.payload.bytes;
    });
    return () => { un.then((f) => f()); };
  });

  async function refresh() {
    launchers = await invoke<Launcher[]>("detect_launchers");
    sources = settings.launchers.curseforge ? await invoke<Source[]>("sync_sources", { extraDirs: settings.extraDirs }) : [];
  }

  async function syncOne(s: Source) {
    syncing[s.path] = 0;
    try {
      const id = await invoke<string>("sync_profile", { path: s.path, extraDirs: settings.extraDirs });
      toast(`${s.name} ${s.syncedAs ? "re-synced" : "imported"} as “${id}”.`);
    } catch (e) {
      toast(`${s.name}: ${e}`);
    }
    delete syncing[s.path];
    await syncInstances();
    await refresh();
  }

  async function syncAll() {
    for (const s of sources) await syncOne(s);
  }

  async function addFolder() {
    const dir = await open({ directory: true, title: "Folder that contains CurseForge-style instances" });
    if (typeof dir === "string" && !settings.extraDirs.includes(dir)) {
      settings.extraDirs = [...settings.extraDirs, dir];
      await saveSettings();
      await refresh();
    }
  }

  /** The mode every profile shares (highlighted), null when they differ. */
  let allMode = $state<string | null>(null);
  /** Where Obelisk keeps its own data. */
  let dataDir = $state<{ dir: string; default: string } | null>(null);
  invoke<{ dir: string; default: string }>("data_dir_get").then((d) => (dataDir = d)).catch(() => {});
  async function moveData(dir?: string) {
    const to = dir ?? (await open({ directory: true, title: "Where should Obelisk keep its data?" }));
    if (!to || typeof to !== "string") return;
    if (!(await ask(`Move Obelisk's data to ${to}? Obelisk closes, moves it and opens again.`, { title: "Data folder" }))) return;
    try {
      await invoke("data_dir_set", { dir: to });
    } catch (e) {
      toast(String(e));
    }
  }
  /** Where the modpacks live (Prism's instances folder). */
  let instDir = $state<{ dir: string; default: string } | null>(null);
  invoke<{ dir: string; default: string }>("instances_dir_get").then((d) => (instDir = d)).catch(() => {});
  let instMove = $state<string | null>(null);
  async function moveInstances(dir?: string) {
    const to = dir ?? (await open({ directory: true, title: "Where should your modpacks live?" }));
    if (!to || typeof to !== "string") return;
    if (!(await ask(`Move every modpack to ${to}? Close Prism Launcher and any running game first. A move to another drive copies everything, checks the copy, then removes the old files; it can take a while for big packs.`, { title: "Modpacks folder" }))) return;
    const gb = (n: number) => (n / 1073741824).toFixed(1);
    instMove = "Starting…";
    const stop = await listen<{ done: number; total: number }>("instances-move", (e) => {
      instMove = `Moving… ${gb(e.payload.done)} of ${gb(e.payload.total)} GB`;
    });
    try {
      const left = await invoke<string[]>("instances_dir_set", { dir: to });
      toast(left.length ? `Moved, but couldn't remove some old files: ${left.join("; ")}` : `Your modpacks are now in ${to}.`);
      instDir = await invoke("instances_dir_get");
      await syncInstances();
    } catch (e) {
      toast(String(e));
    } finally {
      stop();
      instMove = null;
    }
  }
  invoke<string | null>("window_mode_common").then((m) => (allMode = m)).catch(() => {});
  async function setAllWindows(mode: string, label: string) {
    try {
      const n = await invoke<number>("set_window_mode_all", { mode });
      allMode = mode;
      toast(`${label} set for ${n} profile${n === 1 ? "" : "s"}.`);
    } catch (e) { toast(String(e)); }
  }
  async function saveDefaults() {
    await invoke("set_prism_defaults", { defaults: $state.snapshot(defaults) });
    toast("Saved Prism's default Minecraft and Java settings.");
  }

  async function saveKey() {
    await invoke("set_cf_key", { key: keyInput });
    hasKey = await invoke<boolean>("has_cf_key");
    keyInput = "";
    resetCfSearch();
    toast(hasKey ? "CurseForge key saved to Windows Credential Manager." : "CurseForge key removed.");
  }

  const mb = (n: number) => (n >= 1 << 30 ? `${(n / (1 << 30)).toFixed(1)} GB` : `${Math.round(n / (1 << 20))} MB`);

  let aiBusy = $state(false);
  async function connectAi() {
    aiBusy = true;
    try {
      await invoke<string>("ai_connect", { assistant: settings.aiAssistant });
      toast(`Connected. Start a new ${settings.aiAssistant === "gpt" ? "Codex" : "Claude"} chat to use Obelisk.`);
    } catch (e) {
      toast(String(e));
    }
    aiBusy = false;
  }
</script>

<div class="settings">
  <nav class="side">
    <h2>Settings</h2>
    {#each SECTIONS as [id, label] (id)}
      <button class:on={app.settingsSection === id} onclick={() => (app.settingsSection = id)}>{label}</button>
    {/each}
  </nav>

  <div class="body">
    {#if app.settingsSection === "general"}
      <h1>General</h1>
      <label class="field">Open on start
        <select class="select" bind:value={settings.startPage} onchange={saveSettings}>
          <option value="library">My Modpacks</option>
          <option value="discover">Discover</option>
        </select>
      </label>
      <label class="field">Keep deleted mods, packs and worlds in a profile's trash for
        <select class="select" bind:value={settings.trashHours} onchange={saveSettings}>
          {#each [[24, "24 hours"], [72, "3 days"], [168, "1 week"], [336, "2 weeks"], [720, "30 days"], [2160, "90 days"]] as [h, l] (h)}<option value={h}>{l}</option>{/each}
        </select>
        <small class="muted">Each profile has its own trash (the trash button on its content list). After this long, items are deleted for good.</small>
      </label>
      <label class="field">Keep deleted modpacks in the modpack trash for
        <select class="select" bind:value={settings.packTrashHours} onchange={saveSettings}>
          {#each [[72, "3 days"], [168, "1 week"], [336, "2 weeks"], [720, "30 days"], [2160, "90 days"]] as [h, l] (h)}<option value={h}>{l}</option>{/each}
        </select>
        <small class="muted">Restore them from the trash button on My Modpacks.</small>
      </label>
      <div class="field">Backups
        <label class="row-check"><input type="checkbox" bind:checked={settings.backups.auto} onchange={saveSettings} /> Back up each modpack at noon and midnight when it changed</label>
        <label class="row-check">Keep <input class="num" type="number" min="1" max="200" bind:value={settings.backups.keepAuto} onchange={saveSettings} /> automatic backups and
          <input class="num" type="number" min="1" max="500" bind:value={settings.backups.keepManual} onchange={saveSettings} /> checkpoints per modpack</label>
        <small class="muted">Backups store only what changed (worlds aren't included). See them under right-click → Backups.</small>
      </div>
      <label class="toggle-row">
        <span><strong>Keep running in the tray</strong><small>Closing the window keeps Obelisk in the system tray, so games keep their Stop button and shared packs keep syncing. Quit from the tray icon.</small></span>
        <button class="switch" class:on={settings.closeToTray} role="switch" aria-checked={settings.closeToTray} aria-label="Keep running in the tray"
          onclick={() => { settings.closeToTray = !settings.closeToTray; saveSettings(); }}><i></i></button>
      </label>
      <label class="toggle-row">
        <span><strong>Start with Windows</strong><small>Opens Obelisk quietly in the tray when you sign in to Windows.</small></span>
        <button class="switch" class:on={settings.startWithWindows} role="switch" aria-checked={settings.startWithWindows} aria-label="Start with Windows"
          onclick={async () => { const on = !settings.startWithWindows; try { await invoke("set_autostart", { on }); settings.startWithWindows = on; saveSettings(); } catch (e) { toast(String(e)); } }}><i></i></button>
      </label>
      <label class="toggle-row">
        <span><strong>Quiet game start</strong><small>Play just starts the game. Turn off to see Prism Launcher's download and progress windows. Sign-in prompts and errors always show.</small></span>
        <button class="switch" class:on={settings.quietLaunch} role="switch" aria-checked={settings.quietLaunch} aria-label="Quiet game start"
          onclick={() => { settings.quietLaunch = !settings.quietLaunch; saveSettings(); }}><i></i></button>
      </label>
      <label class="toggle-row">
        <span><strong>Offer Fabric mods on NeoForge</strong><small>Shows “Via Connector” for Fabric-only mods (needs Sinytra Connector and Forgified Fabric API).</small></span>
        <button class="switch" class:on={settings.connector} role="switch" aria-checked={settings.connector} aria-label="Offer Fabric mods on NeoForge"
          onclick={() => { settings.connector = !settings.connector; saveSettings(); }}><i></i></button>
      </label>
      <div class="field">Modpacks folder
        <p class="muted">Where every modpack lives{instDir ? `: ${instDir.dir}` : ""}.</p>
        <div class="row-buttons">
          {#if instMove}<span>{instMove}</span>
          {:else}
            <button class="ghost" onclick={() => moveInstances()}>Change…</button>
            {#if dataDir && instDir && instDir.dir.toLowerCase() !== `${dataDir.dir}\\Instances`.toLowerCase()}<button class="ghost" onclick={() => moveInstances(`${dataDir!.dir}\\Instances`)}>Keep them with Obelisk's data</button>{/if}
          {/if}
        </div>
      </div>
      <div class="field">Data folder
        <p class="muted">Obelisk's own settings, caches and sharing key{dataDir ? `: ${dataDir.dir}` : ""}. The modpacks folder is set above.</p>
        <div class="row-buttons">
          <button class="ghost" onclick={() => moveData()}>Change…</button>
          {#if dataDir && dataDir.dir !== dataDir.default}<button class="ghost" onclick={() => moveData(dataDir!.default)}>Back to the default</button>{/if}
        </div>
      </div>
      <div class="field">Accounts
        <p class="muted">Prism signs you in to Minecraft. Use the avatar menu to view your profile, or manage and log out of accounts in Prism.</p>
        <div><button class="ghost" onclick={() => invoke("open_prism")}>Open Prism accounts</button></div>
      </div>

    {:else if app.settingsSection === "sync"}
      <h1>Sync & Import</h1>
      <p class="muted">Bring profiles from other launchers into Obelisk. The first sync copies everything (worlds, configs, mods) into a new profile and carries over play time, memory and icons. Later syncs refresh mods, resource packs, shaders and configs, and never touch your worlds.</p>

      <h3>Launchers</h3>
      <div class="launchers">
        {#each launchers as l (l.id)}
          <label class="launcher" class:dim={!l.found}>
            <input type="checkbox" disabled={!l.found || !l.supported} checked={!!settings.launchers[l.id]}
              onchange={async (e) => { settings.launchers = { ...settings.launchers, [l.id]: e.currentTarget.checked }; await saveSettings(); refresh(); }} />
            <span><strong>{l.name}</strong>
              <small>{l.found ? (l.supported ? "Found" : "Found · sync support coming") : "Not found"}</small></span>
          </label>
        {/each}
      </div>
      <div class="dirs">
        {#each settings.extraDirs as d (d)}
          <span class="dir">{d} <button class="link" onclick={async () => { settings.extraDirs = settings.extraDirs.filter((x) => x !== d); await saveSettings(); refresh(); }}>remove</button></span>
        {/each}
        <button class="ghost" onclick={addFolder}>+ Add an instances folder</button>
      </div>

      <div class="sync-head">
        <h3>Profiles found ({sources.length})</h3>
        <button class="primary" disabled={!sources.length || Object.keys(syncing).length > 0} onclick={syncAll}>Sync all</button>
      </div>
      <div class="sources">
        {#each sources as s (s.path)}
          <div class="source">
            <div><strong>{s.name}</strong>
              <small>{LOADER_NAMES[s.loaderUid] ?? "Vanilla"} {s.loaderVersion} · {s.minecraft} · {Math.round(s.secondsPlayed / 3600)} h · {s.path}</small></div>
            <span class="status" class:ok={!!s.syncedAs}>{s.syncedAs ? `Synced as “${s.syncedAs}”` : "Not in Obelisk yet"}</span>
            <button class="ghost" disabled={s.path in syncing} onclick={() => syncOne(s)}>
              {s.path in syncing ? `Copying… ${mb(syncing[s.path])}` : s.syncedAs ? "Re-sync" : "Sync"}
            </button>
          </div>
        {:else}
          <p class="muted">No profiles found in the selected launchers.</p>
        {/each}
      </div>

    {:else if app.settingsSection === "themes"}
      <h1>Themes</h1>
      {#each ["Standard", "Block"] as group (group)}
        <h3 class="tgroup">{group === "Block" ? "Block themes" : "Standard"}</h3>
        {#if group === "Block"}<p class="muted small">Pixel caves behind the app, blocky headings and glowing buttons.</p>{/if}
        <div class="themes">
          {#each THEMES.filter((t) => t.group === group) as t (t.id)}
            <button class="theme" class:light={t.light} class:on={settings.theme === t.id} onclick={() => { settings.theme = t.id; saveSettings(); }}>
              <span class="sw" style:background={t.swatch}></span>
              <span class="bars"><i></i><i></i></span>
              <span class="tname">{t.label}{#if settings.theme === t.id} ✓{/if}</span>
              {#if t.blurb}<span class="tblurb">{t.blurb}</span>{/if}
            </button>
          {/each}
        </div>
      {/each}

    {:else if app.settingsSection === "minecraft"}
      <h1>Minecraft & Java</h1>
      <p class="muted">Defaults for every profile in Prism. A profile's own options (right-click → Profile Options) override these.</p>
      {#if prismOpen}<p class="warn">Prism is open. Close it before saving, or it may overwrite these settings when it exits.</p>{/if}
      {#if defaults}
        <div class="field">Allocated memory
          <div class="mem">
            <input type="range" min="1024" max={ram} step="256" bind:value={defaults.maxMemMb} aria-label="Default maximum memory" />
            <input class="num" type="number" min="512" max={ram} step="256" bind:value={defaults.maxMemMb} aria-label="Default maximum memory in MB" /> MB
          </div>
          <small class="muted">{(defaults.maxMemMb / 1024).toFixed(1)} GB of {(ram / 1024).toFixed(0)} GB installed</small>
        </div>
        <label class="field">JVM arguments <textarea class="args" bind:value={defaults.jvmArgs} spellcheck="false"></textarea></label>
        <div class="field">Game window
          <Resolution bind:width={defaults.width} bind:height={defaults.height} bind:maximized={defaults.maximized} />
        </div>
        <div><button class="primary" onclick={saveDefaults}>Save</button></div>
        <div class="field">Window mode for every profile
          <div class="seg">
            {#each [["windowed", "Windowed"], ["borderless", "Borderless"], ["fullscreen", "Fullscreen"]] as [m, l] (m)}
              <button class:on={allMode === m} aria-pressed={allMode === m} onclick={() => setAllWindows(m, l)}>{l}</button>
            {/each}
          </div>
          <small class="muted">Sets every profile at once. You can still change one profile later in its Profile Options. Applies next time you play.</small>
        </div>
      {/if}

    {:else if app.settingsSection === "plugins"}
      <h1>Plugins</h1>
      <p class="muted">Optional extras that hook Obelisk up to mods you use. They stay off until you tick them.</p>
      <label class="toggle-row">
        <span>
          <strong>Lightning</strong>
          <small>Shows your Lightning profiler captures in each profile's Diagnosis tab: FPS, slow frames, memory and which mods cost the most, with a history of recent runs. Works in profiles that have Lightning installed.</small>
        </span>
        <button class="switch" class:on={settings.plugins.lightning} role="switch" aria-checked={settings.plugins.lightning} aria-label="Lightning plugin"
          onclick={() => { settings.plugins = { ...settings.plugins, lightning: !settings.plugins.lightning }; saveSettings(); }}><i></i></button>
      </label>
      <label class="toggle-row">
        <span>
          <strong>AI assistant</strong>
          <small>Lets Claude or ChatGPT (through Codex) work with Obelisk: launch test copies on your test bench, switch mods there, and read crash diagnoses and logs. Only from this PC. Your play instance can be read but is never launched or changed. Every request is logged in the test bench's obelisk/audit.log.</small>
        </span>
        <button class="switch" class:on={settings.controlApi} role="switch" aria-checked={settings.controlApi} aria-label="AI assistant plugin"
          onclick={() => { settings.controlApi = !settings.controlApi; saveSettings(); }}><i></i></button>
      </label>
      {#if settings.controlApi}
        <div class="ai-row">
          <div class="seg" role="radiogroup" aria-label="Assistant">
            {#each [["claude", "Claude"], ["gpt", "ChatGPT (Codex)"]] as [v, l]}
              <button role="radio" aria-checked={settings.aiAssistant === v} class:on={settings.aiAssistant === v}
                onclick={() => { settings.aiAssistant = v as "claude" | "gpt"; saveSettings(); }}>{l}</button>
            {/each}
          </div>
          <button class="primary" disabled={aiBusy} onclick={connectAi}>{aiBusy ? "Connecting…" : "Connect"}</button>
        </div>
        <p class="muted small">Connect adds Obelisk to {settings.aiAssistant === "gpt" ? "Codex" : "Claude Code"} as a tool. Start a new chat afterwards to see it. Keep Obelisk open (the tray is enough) while the assistant works.</p>
      {/if}
    {:else if app.settingsSection === "curseforge"}
      <h1>CurseForge API</h1>
      <p class="muted">Obelisk uses Modrinth out of the box. To also search CurseForge, open CurseForge-only mod pages, and update CurseForge mods, add your own API key. It's free from the CurseForge for Studios console. Your key is stored in Windows Credential Manager on this PC only. Obelisk never shows it again or sends it anywhere except CurseForge's API.</p>
      <div><button class="link" onclick={() => openUrl("https://console.curseforge.com/")}>Get a key at console.curseforge.com ↗</button></div>
      <p class="status-line">{hasKey ? "✓ A key is saved." : "No key saved."}</p>
      <div class="keyrow">
        <input class="search" type="password" placeholder={hasKey ? "Paste a new key to replace it" : "Paste your API key"} bind:value={keyInput} autocomplete="off" />
        <button class="primary" disabled={!keyInput.trim()} onclick={saveKey}>Save key</button>
        {#if hasKey}<button class="ghost" onclick={() => { keyInput = ""; saveKey(); }}>Remove</button>{/if}
      </div>

    {:else}
      <h1>About</h1>
      <p><strong>Obelisk</strong>{#if version} {version}{/if}: a fast front end for Prism Launcher.</p>
      <div class="row-buttons update-row">
        {#if found}
          <button class="primary" disabled={updateState === "installing"} onclick={installUpdate}>{updateState === "installing" ? "Updating…" : `Update to ${found.version}`}</button>
          <span class="muted">{found.notes}</span>
        {:else}
          <button class="ghost" disabled={updateState === "checking"} onclick={checkUpdates}>{updateState === "checking" ? "Checking…" : "Check for updates"}</button>
          {#if updateState === "current"}<span class="muted">You have the newest Obelisk.</span>{/if}
        {/if}
      </div>
      <p class="muted">Not affiliated with CurseForge, Overwolf, Modrinth or Mojang. Prism Launcher handles accounts, Java and launching.</p>
    {/if}
  </div>
</div>

<style>
  .settings { flex: 1; min-height: 0; display: grid; grid-template-columns: 230px 1fr; gap: 28px; padding-top: 18px; }
  .side { display: flex; flex-direction: column; gap: 2px; }
  .side h2 { font-size: 18px; margin: 0 0 12px 10px; }
  .side button { text-align: left; background: none; border: 0; padding: 9px 12px; border-radius: 6px; color: var(--text-2); }
  .side button:hover { background: var(--panel); }
  .side button.on { background: var(--panel-2); color: var(--text); font-weight: 600; }
  .body { overflow-y: auto; padding: 0 10px 40px 0; display: flex; flex-direction: column; gap: 16px; max-width: 820px; }
  h1 { margin: 0; font-size: 22px; }
  h3 { margin: 6px 0 0; font-size: 15px; }
  .muted { color: var(--muted); margin: 0; line-height: 1.5; }
  p { margin: 0; }
  .field { display: flex; flex-direction: column; gap: 8px; font-weight: 600; }
  .field .select { width: 260px; font-weight: 400; }
  .toggle-row { display: flex; align-items: center; justify-content: space-between; gap: 20px; background: var(--panel); padding: 12px 14px; border-radius: 8px; }
  .toggle-row small { display: block; color: var(--muted); font-weight: 400; }
  .switch { flex: none; width: 40px; height: 22px; border-radius: 999px; border: 0; padding: 0; position: relative; background: var(--switch-off); transition: background 0.15s; }
  .switch i { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform 0.15s; }
  .switch.on { background: var(--accent); }
  .switch.on i { transform: translateX(18px); }
  .launchers { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 8px; }
  .launcher { display: flex; gap: 10px; align-items: center; background: var(--panel); padding: 10px 12px; border-radius: 8px; }
  .launcher.dim { opacity: 0.55; }
  .launcher small, .source small { display: block; color: var(--muted); font-size: 12px; font-weight: 400; }
  .dirs { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
  .dir { background: var(--panel); padding: 6px 10px; border-radius: 6px; font-size: 13px; }
  .sync-head { display: flex; align-items: center; justify-content: space-between; }
  .sources { display: flex; flex-direction: column; gap: 6px; }
  .source { display: grid; grid-template-columns: 1fr auto auto; gap: 16px; align-items: center; background: var(--panel); padding: 10px 14px; border-radius: 8px; }
  .source small { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 460px; }
  .status { color: var(--muted); font-size: 13px; }
  .status.ok { color: var(--mr); }
  .themes { display: flex; flex-wrap: wrap; gap: 12px; }
  .theme { width: 200px; display: grid; grid-template-columns: 34px 1fr; grid-template-rows: auto auto; gap: 6px 10px; align-items: center; background: #1f2024; border: 2px solid transparent; border-radius: 8px; padding: 12px; color: #e9e9ec; text-align: left; }
  .theme.light { background: #ffffff; color: #18181c; border-color: #d9d9e0; }
  .theme.on { border-color: var(--accent); }
  .sw { width: 30px; height: 30px; border-radius: 6px; grid-row: span 1; }
  .bars { display: flex; flex-direction: column; gap: 5px; }
  .bars i { display: block; height: 7px; border-radius: 4px; background: #5a5b63; }
  .bars i + i { width: 70%; background: #3a3b42; }
  .theme.light .bars i { background: #c4c4cc; }
  .theme.light .bars i + i { background: #e0e0e6; }
  .tblurb { grid-column: 1 / -1; font-size: 12px; color: #9a9ba3; }
  .tgroup { margin: 18px 0 6px; font-size: 15px; }
  .tname { grid-column: span 2; font-weight: 600; }
  .warn { background: var(--err-bg); color: var(--err-text); padding: 8px 10px; border-radius: 6px; }
  .mem { display: flex; align-items: center; gap: 10px; font-weight: 400; }
  .mem input[type="range"] { flex: 1; max-width: 420px; accent-color: var(--accent); }
  .row-check { display: flex; align-items: center; gap: 8px; font-weight: 400; flex-wrap: wrap; }
  .num { width: 90px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 5px; padding: 5px 6px; font: inherit; }
  .args { min-height: 80px; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px; font: 12.5px "Cascadia Mono", Consolas, monospace; font-weight: 400; resize: vertical; }
  .keyrow { display: flex; gap: 10px; }
  .keyrow .search { width: 380px; }
  .status-line { font-weight: 600; }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 8px 20px; font-weight: 600; }
  .primary:disabled { opacity: 0.5; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; }
  .ghost:disabled { opacity: 0.6; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; }
  .ai-row { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; margin: -4px 0 0; }
  .seg { display: inline-flex; width: fit-content; margin: 4px 0; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .seg button { padding: 6px 12px; background: transparent; border: 0; color: inherit; cursor: pointer; }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button:hover { background: var(--hover, rgba(127,127,127,.12)); }
  .seg button.on { background: var(--accent); color: #fff; box-shadow: inset 0 0 0 2px rgb(255 255 255 / 0.35); font-weight: 600; }
  .small { font-size: 12px; }
  .row-buttons { display: flex; gap: 8px; }
  .update-row { align-items: center; gap: 12px; }
</style>
