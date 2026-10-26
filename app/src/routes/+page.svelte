<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { installDevMock } from "$lib/devmock";
  import "@fontsource/pixelify-sans/500.css";
  import "@fontsource/pixelify-sans/400.css";
  import "@fontsource/pixelify-sans/600.css";
  import "@fontsource/pixelify-sans/700.css";
  import "$lib/themes.css";
  import "@fontsource/nunito/600.css";
  import "@fontsource/nunito/700.css";
  import "@fontsource/nunito/800.css";
  import "$lib/art.css";
  import { drawScene } from "$lib/scene";
  import { runParticles } from "$lib/particles";
  import PackMenu, { openPackMenu } from "$lib/PackMenu.svelte";
  import { blockTheme } from "$lib/themes";
  import Icon from "$lib/Icon.svelte";
  import { backgroundCheck, recheck } from "$lib/updates.svelte";
  if (import.meta.env.DEV) installDevMock();
  import { open } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { api, initials, type Instance } from "$lib/api";
  import { app, openBrowse, toast, closeOverlays, type Page, type Overlay } from "$lib/state.svelte";
  import { syncInstances } from "$lib/install";
  import { startAssistantBridge } from "$lib/assistant.svelte";
  import Library from "$lib/Library.svelte";
  import InstanceView from "$lib/InstanceView.svelte";
  import Discover from "$lib/Discover.svelte";
  import Browse from "$lib/Browse.svelte";
  import ProjectPage from "$lib/ProjectPage.svelte";
  import AuthorPage from "$lib/AuthorPage.svelte";
  import InstallDialogs from "$lib/InstallDialogs.svelte";
  import ProfileDialogs from "$lib/ProfileDialogs.svelte";
  import ServerPack from "$lib/ServerPack.svelte";
  import SharePack from "$lib/SharePack.svelte";
  import PackShare from "$lib/PackShare.svelte";
  import PackImport from "$lib/PackImport.svelte";
  import SettingsPage from "$lib/SettingsPage.svelte";
  import Welcome from "$lib/Welcome.svelte";
  import WhatsNew from "$lib/WhatsNew.svelte";
  import WorkBar from "$lib/WorkBar.svelte";
  import { loadSettings, settings } from "$lib/settings.svelte";
  import { refreshRunning } from "$lib/play.svelte";

  // One-click updates of Obelisk itself, from its GitHub releases (Obelisk also checks every 30 minutes by itself).
  type SelfUpdate = { version: string; notes: string };
  let selfUpdate = $state<SelfUpdate | null>(null);
  // Shown once after an update installed this version.
  let whatsNew = $state<SelfUpdate | null>(null);
  let updating = $state(false);
  let lastCheck = 0;
  async function checkSelfUpdate() {
    if (Date.now() - lastCheck < 15 * 60_000) return;
    lastCheck = Date.now();
    selfUpdate = (await invoke<SelfUpdate | null>("self_update_check").catch(() => null)) ?? selfUpdate;
  }
  listen<SelfUpdate>("self-update", (e) => (selfUpdate = e.payload));
  async function installSelfUpdate() {
    updating = true;
    try {
      await invoke("self_update_install");
    } catch (e) {
      updating = false;
      toast(String(e));
    }
  }

  let account = $state<{ name: string; uuid: string } | null>(null);
  let accountMenu = $state(false);
  let accounts = $state<{ name: string; uuid: string; active: boolean }[]>([]);

  async function loadAccounts() {
    accounts = await invoke("list_accounts");
    account = accounts.find((a) => a.active) ?? accounts[0] ?? null;
  }

  /** Switch or log out directly in Prism's account list (Prism must be closed). */
  async function accountAction(cmd: string, uuid: string, done: string) {
    accountMenu = false;
    try {
      await invoke(cmd, { uuid });
      await loadAccounts();
      toast(done);
    } catch (e) {
      toast(String(e));
    }
  }

  const NAV: { page: Page; label: string; icon: string }[] = [
    { page: "discover", label: "Discover", icon: "compass" },
    { page: "browse", label: "Browse", icon: "chest" },
  ];

  let showWelcome = $state(false);
  let resizeTimer: ReturnType<typeof setTimeout>;
  let searchEl = $state<HTMLInputElement>();
  // Block themes paint a pixel cave behind the app (redrawn when the window size or theme changes).
  let sceneEl = $state<HTMLCanvasElement>();
  let fxEl = $state<HTMLCanvasElement>();
  $effect(() => {
    if (fxEl) runParticles(fxEl, blockTheme(settings.theme)?.scene ?? null);
  });
  let size = $state(0);
  $effect(() => {
    void size;
    const bt = blockTheme(settings.theme);
    if (sceneEl) drawScene(sceneEl, bt?.art ? null : bt?.scene ?? null);
  });
  $effect(() => {
    if (app.focusSearch) queueMicrotask(() => searchEl?.focus());
  });

  onMount(async () => {
    await loadSettings();
    startAssistantBridge();
    // Started from inside another app's sandbox (e.g. the Claude app): games would load slowly and see stale files.
    if (await invoke<boolean>("sandboxed").catch(() => false))
      toast("Obelisk was started from inside another app, so Windows is redirecting its files. Close it and start Obelisk from the Start menu for normal speed.");
    refreshRunning();
    whatsNew = await invoke<SelfUpdate | null>("whats_new").catch(() => null);
    checkSelfUpdate();
    app.page = settings.startPage;
    // People who already saved a key don't need the how-to.
    if (!settings.welcomed && (await invoke<boolean>("has_cf_key").catch(() => false))) settings.welcomed = true;
    showWelcome = !settings.welcomed;
    loadAccounts();
    await syncInstances();
    // Update checks run quietly in the background: now, then every 12 hours.
    backgroundCheck(app.instances);
  });
  $effect(() => {
    const timer = setInterval(() => backgroundCheck(app.instances), 12 * 60 * 60 * 1000);
    return () => clearInterval(timer);
  });

  // Shared packs: badges for friends' changes waiting for review (the backend pulls every minute).
  const refreshShares = () => invoke<Record<string, number>>("share_overview").then((m) => (app.shareIncoming = m)).catch(() => {});
  $effect(() => {
    refreshShares();
    let un: (() => void) | undefined;
    import("@tauri-apps/api/event").then(({ listen }) => listen("share-incoming", (e) => {
      refreshShares();
      toast(`Friends sent ${e.payload} change${e.payload === 1 ? "" : "s"} to a shared pack. Right-click it → Collaborative Pack to review.`);
    })).then((u) => (un = u)).catch(() => {});
    window.addEventListener("share-changed", refreshShares);
    return () => { un?.(); window.removeEventListener("share-changed", refreshShares); };
  });

  // Automatic backups: one per noon/midnight slot for each pack that changed. Runs a minute after
  // start (catching up a slot missed while Obelisk was closed), then at every noon and midnight.
  const slotStart = (d = new Date()) => { const s = new Date(d); s.setHours(s.getHours() >= 12 ? 12 : 0, 0, 0, 0); return s.getTime(); };
  const nextSlot = (d = new Date()) => { const s = new Date(slotStart(d)); s.setHours(s.getHours() + 12); return s.getTime(); };
  async function autoBackups() {
    if (!settings.backups.auto) return;
    const slotMs = slotStart();
    for (const inst of app.instances) await invoke("auto_backup", { instanceId: inst.id, slotMs }).catch(() => {});
  }
  $effect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const plan = () => { timer = setTimeout(() => { autoBackups(); plan(); }, nextSlot() - Date.now() + 5000); };
    // And a backup of any pack that changed while Obelisk was closed (no change, no backup).
    const first = setTimeout(async () => {
      if (settings.backups.auto) for (const inst of app.instances) await invoke("change_backup", { instanceId: inst.id, label: "Changed while Obelisk was closed" }).catch(() => {});
      autoBackups();
    }, 60_000);
    plan();
    return () => { clearTimeout(timer); clearTimeout(first); };
  });

  function go(page: Page) {
    closeOverlays();
    app.page = page;
  }

  const win = getCurrentWindow();

  // Closing the window quits Obelisk when it isn't kept in the tray, which ends any sharing under way.
  // (With the tray on, the app hides the window itself; a listener here must then not let it be destroyed.)
  onMount(() => {
    const un = win.onCloseRequested((e) => {
      if (settings.closeToTray) return e.preventDefault();
      const sharing = Object.values(app.packCodes).some((c) => c.expiresMs > Date.now());
      if (sharing && !confirm("You're sharing a pack. Closing Obelisk stops it, and a friend who is still downloading won't get the rest. Close anyway?")) e.preventDefault();
    });
    return () => { un.then((u) => u()); };
  });

  /** Search as you type: jumps to Browse on the first keystroke; Browse debounces the request. */
  function liveSearch(value: string) {
    if (app.page !== "browse" || app.stack.length) openBrowse(app.page === "browse" ? app.browseType : "mod", value);
    else app.browseQuery = value;
  }

  function submitSearch(e: SubmitEvent) {
    e.preventDefault();
    liveSearch(app.browseQuery);
  }

  async function importPack() {
    const path = await open({ title: "Import a modpack", filters: [{ name: "Modpacks", extensions: ["zip", "mrpack"] }] });
    if (typeof path === "string") {
      await api.importPack(path);
      toast("Opened in Prism's importer. Press Sync on My Modpacks when it finishes.");
    }
  }

  // ---- Back/forward history (mouse buttons 4/5, Alt+arrows) ----
  type Snap = { page: Page; current: Instance | null; stack: Overlay[] };
  let past: Snap[] = [];
  let future: Snap[] = [];
  let last: Snap | null = null;
  let restoring = false;

  $effect(() => {
    const snap: Snap = { page: app.page, current: app.current, stack: [...app.stack] };
    if (restoring) { restoring = false; last = snap; return; }
    if (last && (last.page !== snap.page || last.current !== snap.current || last.stack.length !== snap.stack.length || last.stack.at(-1) !== snap.stack.at(-1))) {
      past.push(last);
      if (past.length > 100) past.shift();
      future = [];
    }
    last = snap;
  });

  function travel(from: Snap[], to: Snap[]) {
    const snap = from.pop();
    if (!snap || !last) return;
    to.push(last);
    restoring = true;
    app.page = snap.page;
    app.current = snap.current;
    app.stack = snap.stack;
  }

  function onMouseUp(e: MouseEvent) {
    if (e.button === 3) { e.preventDefault(); travel(past, future); }
    if (e.button === 4) { e.preventDefault(); travel(future, past); }
  }

  function onKey(e: KeyboardEvent) {
    // Ctrl+R / F5: refresh Obelisk's data instead of reloading the window.
    if ((e.ctrlKey && e.key.toLowerCase() === "r") || e.key === "F5") {
      e.preventDefault();
      syncInstances();
      window.dispatchEvent(new Event("obelisk-refresh"));
      if (app.page === "instance" && app.current) {
        recheck(app.current.id).then(() => toast("Checked for updates.")).catch((err) => toast(String(err)));
      }
      return;
    }
    if (e.altKey && e.key === "ArrowLeft") travel(past, future);
    if (e.altKey && e.key === "ArrowRight") travel(future, past);
  }

  // The webview's own menu (save image, copy link...) isn't useful here; keep it only for text fields.
  function onContextMenu(e: MouseEvent) {
    const t = e.target as HTMLElement;
    if (!t.closest("input, textarea, [contenteditable]")) e.preventDefault();
  }
</script>

<!-- Re-read Prism's instances when the window regains focus, so packs imported or changed in Prism show up. -->
<svelte:window onresize={() => { clearTimeout(resizeTimer); resizeTimer = setTimeout(() => size++, 150); }} onmouseup={onMouseUp} onkeydown={onKey} oncontextmenu={onContextMenu} onclick={() => (accountMenu = false)} onfocus={() => { syncInstances(); loadAccounts(); refreshRunning(); checkSelfUpdate(); }} />
<svelte:document onmousedown={(e) => (e.button === 3 || e.button === 4) && e.preventDefault()} />

<canvas class="scene" bind:this={sceneEl} aria-hidden="true"></canvas>
<canvas class="scene fx" bind:this={fxEl} aria-hidden="true"></canvas>
<div class="shell">
  <nav class="rail">
    <button class="rail-btn logo" class:active={app.page === "library"} title="My Modpacks" onclick={() => go("library")}>
      <img src="/logo.png" alt="" width="30" height="30" draggable="false" />
    </button>
    {#each app.instances as inst (inst.id)}
      <button class="rail-item" class:active={app.page === "instance" && app.current?.id === inst.id} title={inst.name}
        onclick={() => { closeOverlays(); app.current = inst; app.page = "instance"; }} oncontextmenu={(e) => openPackMenu(e, inst)}>
        {#if app.icons[inst.id]}<img src={app.icons[inst.id]} alt="" draggable="false" />{:else}<span>{initials(inst.name)}</span>{/if}
        {#if app.shareIncoming[inst.id]}<b class="share-dot" title="Changes from friends to review">{app.shareIncoming[inst.id]}</b>{/if}
      </button>
    {/each}
    <button class="rail-btn" title="Import a modpack" onclick={importPack}>
      <svg viewBox="0 0 24 24" width="22" height="22"><path d="M12 5v14M5 12h14" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
    </button>
    <div class="grow"></div>
    {#if account}
      <div class="acct-wrap">
        <button class="avatar" title={account.name} onclick={(e) => { e.stopPropagation(); accountMenu = !accountMenu; }}>
          <img src="https://mc-heads.net/avatar/{account.uuid}/36" alt={account.name} draggable="false" />
        </button>
        {#if accountMenu}
          <div class="acct-menu" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
            <div class="who"><strong>{account.name}</strong><small>Minecraft account</small></div>
            <button onclick={() => { accountMenu = false; openUrl(`https://namemc.com/profile/${account!.uuid}`); }}>View Profile ↗</button>
            {#each accounts.filter((a) => !a.active) as a (a.uuid)}
              <button onclick={() => accountAction("switch_account", a.uuid, `Switched to ${a.name}.`)}>
                <img class="mini" src="https://mc-heads.net/avatar/{a.uuid}/18" alt="" /> Switch to {a.name}</button>
            {/each}
            <button onclick={() => { accountMenu = false; invoke("open_prism"); toast("Prism's Microsoft sign-in window adds new accounts. Come back here afterwards."); }}>Add account…</button>
            <button class="danger" onclick={() => accountAction("logout_account", account!.uuid, `Logged out of ${account!.name}.`)}>Log out</button>
          </div>
        {/if}
      </div>
    {/if}
    <button class="rail-btn" class:active={app.page === "settings"} title="Settings" onclick={() => go("settings")}>
      <svg viewBox="0 0 24 24" width="20" height="20"><path d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" fill="none" stroke="currentColor" stroke-width="1.6" /></svg>
    </button>
  </nav>

  <main>
    <!-- Custom title bar: the empty parts drag the window, double-click maximizes. -->
    <header class="top" data-tauri-drag-region>
      <span class="brand" data-tauri-drag-region>OBELISK</span>
      <nav class="nav">
        {#each NAV as n (n.page)}
          <button class:on={app.page === n.page} onclick={() => go(n.page)}><Icon name={n.icon} /> {n.label}</button>
        {/each}
      </nav>
      <form class="searchbox" class:filled={!!app.browseQuery} onsubmit={submitSearch}>
        <svg viewBox="0 0 24 24" width="16" height="16"><circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="2" /><path d="m20 20-3.5-3.5" stroke="currentColor" stroke-width="2" stroke-linecap="round" /></svg>
        <input bind:this={searchEl} aria-label="Search for Minecraft projects" value={app.browseQuery} oninput={(e) => liveSearch(e.currentTarget.value)} />
        {#if app.browseQuery}<button type="button" class="clear" aria-label="Clear search" onclick={() => liveSearch("")}>✕</button>{/if}
      </form>
      <div class="drag" data-tauri-drag-region></div>
      {#if selfUpdate}
        <button class="self-update" title={`Obelisk ${selfUpdate.version}${selfUpdate.notes ? ": " + selfUpdate.notes : ""}`} disabled={updating} onclick={installSelfUpdate}>
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path d="M12 4v11m0 0-4.5-4.5M12 15l4.5-4.5M5 19h14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
          {updating ? "Updating…" : "Update"}
        </button>
      {/if}
      <div class="winctl">
        <button aria-label="Minimize" onclick={() => win.minimize()}><svg viewBox="0 0 12 12" width="12" height="12"><path d="M2 6h8" stroke="currentColor" stroke-width="1.2" /></svg></button>
        <button aria-label="Maximize" onclick={() => win.toggleMaximize()}><svg viewBox="0 0 12 12" width="12" height="12"><rect x="2.5" y="2.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.2" /></svg></button>
        <button class="close" aria-label="Close" onclick={() => win.close()}><svg viewBox="0 0 12 12" width="12" height="12"><path d="m2.5 2.5 7 7m0-7-7 7" stroke="currentColor" stroke-width="1.2" /></svg></button>
      </div>
    </header>

    <div class="view">
      {#if app.page === "library"}<Library />
      {:else if app.page === "instance" && app.current}<InstanceView />
      {:else if app.page === "discover"}<Discover />
      {:else if app.page === "browse"}<Browse />
      {:else if app.page === "settings"}<SettingsPage />
      {/if}
      {#if app.stack.length}
        {@const top = app.stack[app.stack.length - 1]}
        {#key top}
          {#if top.kind === "project"}<ProjectPage hit={top.hit} />{:else}<AuthorPage name={top.name} cfId={top.cfId} />{/if}
        {/key}
      {/if}
    </div>
  </main>
</div>

<InstallDialogs />
<ProfileDialogs />
{#if app.serverPack}<ServerPack inst={app.serverPack} />{/if}
{#if app.sharePack}<SharePack inst={app.sharePack} />{/if}
{#if app.packShare}<PackShare inst={app.packShare} />{/if}
{#if app.packImport !== null}<PackImport />{/if}
<PackMenu />
{#if showWelcome && !settings.welcomed}<Welcome />{/if}
<WorkBar />
{#if whatsNew}<WhatsNew version={whatsNew.version} notes={whatsNew.notes} close={() => (whatsNew = null)} />{/if}
{#if app.toast}<div class="toast" role="status">{app.toast}</div>{/if}

<style>
  /* Native dropdown lists: themed, with the picked option clearly marked. */
  :global(select option) { background: var(--panel); color: var(--text); }
  :global(select option:checked) { background: var(--accent); color: #fff; font-weight: 600; }
  /* Themes: Obelisk (dark, default) and Obelisk Light. */
  :global(:root) {
    --bg: #111214; --rail: #0b0b0d; --panel: #1a1b1f; --panel-2: #212228; --line: #2a2b31;
    --text: #e9e9ec; --text-2: #c9cad1; --muted: #9a9ba3; --row-line: #222329; --scroll: #34353c; --switch-off: #3a3b42;
    --accent: #9b87f5; --accent-hover: #b3a3ff;
    --err-bg: #3a1d1d; --err-text: #ffb4a6;
    --cf: #f16436; --mr: #1bd96a; --danger: #e5484d;
    color-scheme: dark;
  }
  :global(:root[data-theme="light"]) {
    --bg: #f3f3f6; --rail: #e6e6ec; --panel: #ffffff; --panel-2: #ececf1; --line: #d9d9e0;
    --text: #18181c; --text-2: #3c3d45; --muted: #6b6c75; --row-line: #ececf1; --scroll: #c4c4cc; --switch-off: #c4c4cc;
    --accent: #7a64e8; --accent-hover: #6a52e0; --err-bg: #fde8e6; --err-text: #9b2c1f;
    --cf: #d9531e; --mr: #0f9d58;
    color-scheme: light;
  }
  :global(html, body) {
    margin: 0; height: 100%; background: var(--bg); color: var(--text);
    font: 14px/1.4 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased; user-select: none; overflow: hidden;
  }
  :global(*) { box-sizing: border-box; }
  :global(img) { -webkit-user-drag: none; }
  :global(button) { font: inherit; color: inherit; cursor: pointer; }
  :global(button:disabled) { cursor: default; }
  :global(input[type="checkbox"]) { accent-color: var(--accent); width: 15px; height: 15px; margin: 0; cursor: pointer; }
  :global(.search) { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 7px 10px; font: inherit; outline: none; width: 220px; }
  :global(.search:focus) { border-color: var(--accent); }
  :global(.select) { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; font: inherit; }
  :global(:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }
  :global(::-webkit-scrollbar) { width: 10px; height: 10px; }
  :global(::-webkit-scrollbar-thumb) { background: var(--scroll); border-radius: 6px; border: 2px solid var(--bg); }
  :global(::-webkit-scrollbar-track) { background: transparent; }

  .scene { display: none; position: fixed; inset: 0; width: 100vw; height: 100vh; z-index: 0; pointer-events: none; }
  .shell { position: relative; z-index: 1; display: grid; grid-template-columns: 64px 1fr; height: 100vh; }
  .rail { background: var(--rail); display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 12px 0; border-right: 1px solid var(--line); }
  .rail-btn { width: 44px; height: 44px; display: grid; place-items: center; border: 0; border-radius: 10px; background: none; color: var(--muted); transition: color 0.15s, background 0.15s; }
  .rail-btn:hover:not(:disabled) { color: var(--text); background: var(--panel); }
  .rail-btn.active { color: var(--accent); background: var(--panel); }
  .logo { color: var(--accent); }
  .logo.active { background: var(--panel); }
  .rail-item { width: 44px; height: 44px; padding: 0; border: 2px solid transparent; border-radius: 10px; background: var(--panel-2); overflow: hidden; display: grid; place-items: center; transition: border-color 0.15s; }
  .rail-item img { width: 100%; height: 100%; object-fit: cover; }
  .rail-item span { font-weight: 700; color: var(--muted); }
  .rail-item.active { border-color: var(--accent); }
  .rail-item { position: relative; }
  .share-dot { position: absolute; top: -2px; right: -2px; min-width: 16px; height: 16px; border-radius: 999px; background: var(--accent); color: #fff; font-size: 10px; display: grid; place-items: center; padding: 0 3px; }
  .grow { flex: 1; }
  .acct-wrap { position: relative; }
  .avatar { padding: 0; border: 0; background: none; border-radius: 50%; }
  .avatar img { width: 36px; height: 36px; border-radius: 50%; border: 2px solid var(--accent); image-rendering: pixelated; display: block; }
  .acct-menu { position: absolute; left: calc(100% + 12px); bottom: -6px; z-index: 40; width: 220px; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 6px; box-shadow: 0 12px 34px rgb(0 0 0 / 0.5); }
  .acct-menu::before { content: ""; position: absolute; left: -6px; bottom: 18px; width: 10px; height: 10px; background: var(--panel-2); border-left: 1px solid var(--line); border-bottom: 1px solid var(--line); transform: rotate(45deg); }
  .who { display: flex; flex-direction: column; padding: 6px 10px 8px; border-bottom: 1px solid var(--line); margin-bottom: 4px; }
  .who small { color: var(--muted); }
  .acct-menu button { display: block; width: 100%; background: none; border: 0; padding: 8px 10px; border-radius: 5px; text-align: left; }
  .acct-menu button:hover { background: var(--panel); }
  .acct-menu button { display: flex; align-items: center; gap: 8px; }
  .acct-menu .danger { color: var(--danger); }
  .mini { width: 18px; height: 18px; border-radius: 3px; image-rendering: pixelated; }

  main { position: relative; display: flex; flex-direction: column; min-width: 0; overflow: hidden; }
  .top { display: flex; align-items: center; gap: 18px; padding: 10px 10px 0 28px; height: 64px; margin-bottom: 8px; }
  .drag { flex: 1; align-self: stretch; }
  .clear { background: none; border: 0; color: var(--muted); padding: 2px 4px; }
  .clear:hover { color: var(--text); }
  .self-update { align-self: flex-start; height: 32px; display: flex; align-items: center; gap: 6px; padding: 0 12px; margin-right: 8px; background: var(--accent); color: #fff; border: 0; border-radius: 6px; font-weight: 700; box-shadow: 0 2px 6px rgb(0 0 0 / 0.3); }
  .self-update:hover:not(:disabled) { filter: brightness(1.1); }
  .self-update:disabled { opacity: 0.7; }
  .winctl { display: flex; gap: 6px; align-self: flex-start; }
  .winctl button { width: 38px; height: 32px; display: grid; place-items: center; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; color: var(--text-2); box-shadow: 0 2px 6px rgb(0 0 0 / 0.3); transition: background 0.12s, color 0.12s; }
  .winctl button:hover { background: var(--panel-2); color: var(--text); border-color: var(--accent); }
  .winctl .close:hover { background: #c42b1c; color: #fff; }
  .brand { font-weight: 800; letter-spacing: 0.18em; font-size: 13px; color: var(--muted); }
  /* Compact until focused (or holding a query), then it widens. */
  .searchbox { flex: 0 1 220px; min-width: 0; transition: flex-basis 0.16s ease-out; display: flex; align-items: center; gap: 8px; background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 0 10px; color: var(--muted); }
  .searchbox:focus-within, .searchbox.filled { flex-basis: 560px; }
  .searchbox svg { flex: none; }
  .searchbox:focus-within { border-color: var(--accent); }
  .searchbox input { flex: 1; background: none; border: 0; outline: none; color: var(--text); font: inherit; padding: 8px 0; }
  .nav { display: flex; gap: 4px; flex: none; }
  .nav button { display: flex; align-items: center; gap: 7px; background: none; border: 0; padding: 7px 12px; font-size: 15px; color: var(--muted); border-bottom: 2px solid transparent; transition: color 0.15s; }
  .nav button:hover { color: var(--text); }
  .nav button.on { color: var(--text); font-weight: 600; border-bottom-color: var(--accent); }
  .view { position: relative; flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 0 28px; }
  .toast { position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); background: var(--panel-2); border: 1px solid var(--line); padding: 10px 16px; border-radius: 8px; box-shadow: 0 10px 30px rgb(0 0 0 / 0.45); z-index: 50; max-width: 70vw; animation: up 0.15s ease-out; }
  @keyframes up { from { opacity: 0; transform: translate(-50%, 8px); } }
</style>
