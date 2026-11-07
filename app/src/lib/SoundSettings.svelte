<script lang="ts">
  // Settings → Sound: one panel at a time (music, update sound, alerts, interface sounds), each opening with a card
  // that holds its switch, volume and main options. The last panel opened is remembered on this PC.
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { settings, saveSettings, TYPE_STYLES, JINGLES, TRACKS, CUES, UI_SETS, SOUND_THEMES, SOUND_SLOTS, slotSources, soundSource } from "./settings.svelte";
  import { toastError } from "./state.svelte";
  import { music } from "./player.svelte";
  import { uiSound, typeSound } from "./uisound.svelte";

  type Tab = "music" | "update" | "alerts" | "interface";
  const TABS: [Tab, string, string][] = [
    ["music", "Background Music", "music"], ["update", "Update Sound", "wave"], ["alerts", "Alerts", "bell"], ["interface", "Interface", "pointer"],
  ];
  /** Lucide-style outline icons, 24×24. */
  const ICONS: Record<string, string> = {
    music: "M9 18V5l12-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
    wave: "M2 12h2M6 8v8M10 4v16M14 7v10M18 10v4M22 12h0",
    bell: "M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9M10.3 21a1.9 1.9 0 0 0 3.4 0",
    pointer: "M4 4l7 17 2.5-7.5L21 11z",
    palette: "M12 22a10 10 0 1 1 10-10c0 3-2.5 4-5 4h-2a2 2 0 0 0-1 3.7A1.5 1.5 0 0 1 12 22zM7.5 11.5h0M10.5 7.5h0M15.5 7.5h0",
    keyboard: "M3 6h18v12H3zM7 10h0M11 10h0M15 10h0M7 14h10",
    speaker: "M11 5L6 9H2v6h4l5 4zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14",
    message: "M21 12a8 8 0 0 1-11.8 7L3 21l2-6.2A8 8 0 1 1 21 12z",
    users: "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM22 21v-2a4 4 0 0 0-3-3.9M16 3.1a4 4 0 0 1 0 7.8",
    gamepad: "M6 11h4M8 9v4M15 12h0M18 10h0M17.3 5H6.7a4 4 0 0 0-4 3.6L2 15a3 3 0 0 0 5.5 1.6L9 14h6l1.5 2.6A3 3 0 0 0 22 15l-.7-6.4A4 4 0 0 0 17.3 5z",
    wifi: "M5 12.5a10 10 0 0 1 14 0M8.5 16a5 5 0 0 1 7 0M2 9a15 15 0 0 1 20 0M12 20h0",
  };
  /** The alert events in groups: (title, icon, cue ids). Covers every id in CUES. */
  const GROUPS: [string, string, string[]][] = [
    ["Messages", "message", ["message", "message_sent", "mention"]],
    ["Friends", "users", ["friend_request", "friend_accepted", "friend_online", "joined_world"]],
    ["Games", "gamepad", ["invite", "invite_accepted", "invite_declined"]],
    ["Connection", "wifi", ["disconnected", "reconnected", "error"]],
  ];
  const cueLabel = (id: string) => CUES.find((c) => c[0] === id)?.[1] ?? id;

  const TAB_KEY = "obelisk.soundTab";
  let tab = $state<Tab>(((): Tab => { try { const t = localStorage.getItem(TAB_KEY); if (TABS.some((x) => x[0] === t)) return t as Tab; } catch { /* no storage */ } return "music"; })());
  function pickTab(t: Tab) { tab = t; try { localStorage.setItem(TAB_KEY, t); } catch { /* no storage */ } }

  let search = $state("");
  let closed = $state<Record<string, boolean>>({});
  /** The cue whose "⋯" menu is open. */
  let menuFor = $state("");
  const found = (id: string) => cueLabel(id).toLowerCase().includes(search.trim().toLowerCase());
  const allOn = $derived(CUES.every(([id]) => !settings.cues.muted.includes(id)));

  function setCue(id: string, on: boolean) {
    settings.cues.muted = on ? settings.cues.muted.filter((m) => m !== id) : [...settings.cues.muted, id];
    saveSettings();
  }

  function pickSoundTheme(next: string) {
    if (next === "custom") settings.soundPicks = Object.fromEntries(SOUND_SLOTS.map(([slot]) => [slot, soundSource(slot)]));
    else if (next !== "classic" && settings.updateJingle !== "off") settings.updateJingle = next;
    else if (next === "classic" && SOUND_THEMES.some((t) => t[0] === settings.updateJingle)) settings.updateJingle = JINGLES[0][0];
    settings.soundTheme = next;
    saveSettings();
    uiSound("open", undefined, true);
  }

  /** Custom pick for one slot; switches to custom mode first (keeping every other sound as it was). */
  function pickSlot(slot: string, source: string) {
    if (settings.soundTheme !== "custom") pickSoundTheme("custom");
    settings.soundPicks = { ...settings.soundPicks, [slot]: source };
    saveSettings();
    playSlot(slot);
  }

  function playSlot(slot: string) {
    const source = soundSource(slot);
    if (slot === "type") typeSound("key", "a", true, source);
    else if (CUES.some((c) => c[0] === slot)) invoke("cue_preview", { id: slot, theme: source }).catch(toastError);
    else uiSound(slot as "hover", source, true);
  }

  /** Ticks or unticks one Shuffle entry: `key` is "skip" (Obelisk tracks) or "skipFiles" (own files). */
  function toggleSkip(key: "skip" | "skipFiles", id: string) {
    const list = settings.music[key];
    settings.music[key] = list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
    saveSettings();
  }
  const shuffleItems = $derived<[string, string][]>(settings.music.source === "mine" ? music.files.map((f) => [f, f]) : TRACKS);
  const skipKey = $derived(settings.music.source === "mine" ? "skipFiles" : "skip");
  const showShuffle = $derived(settings.music.source === "mine" ? music.files.length > 0 : settings.music.track === "all");

  /** The last update sound picked, so switching it back on restores it. */
  let lastJingle = settings.updateJingle === "off" ? JINGLES[0][0] : settings.updateJingle;
  const previewJingle = () => invoke("jingle_preview", { name: settings.updateJingle }).catch(toastError);
</script>

{#snippet icon(name: string, size = 18)}
  <svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true"><path d={ICONS[name]} fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
{/snippet}

{#snippet head(ic: string, title: string, desc: string, on: boolean | null, flip: () => void)}
  <div class="card-head">
    <span class="card-icon">{@render icon(ic, 22)}</span>
    <span class="card-title"><strong>{title}</strong><small>{desc}</small></span>
    {#if on !== null}
      <button class="switch" class:on role="switch" aria-checked={on} aria-label={title} onclick={flip}><i></i></button>
    {/if}
  </div>
{/snippet}

{#snippet volume(label: string, value: number, set: (v: number) => void, done: () => void, disabled = false)}
  <label class="vol" class:disabled>
    <span class="vol-label">{label}</span>
    {@render icon("speaker", 16)}
    <input type="range" min="1" max="100" {value} {disabled} aria-label={label} oninput={(e) => set(+e.currentTarget.value)} onchange={done} />
    <span class="pct">{value}%</span>
  </label>
{/snippet}

<div class="sound">
  <h1>Sound</h1>
  <label class="row-check top"><input type="checkbox" bind:checked={settings.muteInactive} onchange={saveSettings} /> Mute when Obelisk isn't the active window
    <small class="muted">Music, the update sound, alerts and menu sounds all hold back while another window is in front.</small></label>

  <div class="tabs" role="tablist" aria-label="Sound">
    {#each TABS as [id, label, ic] (id)}
      <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => pickTab(id)}>{@render icon(ic, 16)}<span>{label}</span></button>
    {/each}
  </div>

  {#if tab === "music"}
    <section class="card">
      {@render head("music", "Background music", "Soft original tracks while you browse, or your own music files.", settings.music.on,
        () => { settings.music.on = !settings.music.on; music.paused = false; saveSettings(); })}
      {#if settings.music.on}
        <div class="card-body">
          {@render volume("Music volume", settings.music.volume, (v) => (settings.music.volume = v), saveSettings)}
          <div class="opt">
            <span class="opt-label">Play</span>
            <div class="seg small">
              <button class:active={settings.music.source === "obelisk"} onclick={() => { settings.music.source = "obelisk"; saveSettings(); }}>Obelisk's music</button>
              <button class:active={settings.music.source === "mine"} onclick={() => { settings.music.source = "mine"; saveSettings(); }}>My own music</button>
            </div>
          </div>
          {#if settings.music.source === "obelisk"}
            <label class="opt"><span class="opt-label">Track</span>
              <select class="select" bind:value={settings.music.track} onchange={saveSettings} aria-label="Track">
                <option value="all">Shuffle</option>
                {#each TRACKS as [id, label] (id)}<option value={id}>{label}</option>{/each}
              </select></label>
          {:else}
            <div class="opt"><span class="opt-label">Folder</span>
              <span class="path" title={settings.music.folder}>{settings.music.folder || "No folder chosen yet."}</span>
              <button class="ghost" onclick={async () => { const dir = await open({ directory: true, title: "Choose your music folder" }); if (typeof dir === "string") { Object.assign(settings.music, { folder: dir, source: "mine", on: true }); music.paused = false; saveSettings(); } }}>{settings.music.folder ? "Change…" : "Choose…"}</button>
            </div>
            <small class="muted">Plays every mp3, ogg, opus, wav, flac and m4a file in it. They stay on this PC.</small>
          {/if}
          <div class="opt">
            <button class="ghost" onclick={() => (music.paused = !music.paused)}>{music.paused ? "Resume" : "Pause"}</button>
            {#if music.title}<small class="muted">Playing {music.title}</small>{/if}
          </div>
          {#if music.error}<small class="muted">{music.error}</small>{/if}
        </div>
      {/if}
    </section>

    {#if settings.music.on && showShuffle}
      <div class="list-head">
        <h2>Shuffle {settings.music.source === "mine" ? "files" : "tracks"}</h2>
        <label class="row-check"><input type="checkbox" checked={settings.music[skipKey].length === 0}
          onchange={(e) => { settings.music[skipKey] = e.currentTarget.checked ? [] : shuffleItems.map((i) => i[0]); saveSettings(); }} /> Select all</label>
      </div>
      <div class="group">
        <div class="rows cols">
          {#each shuffleItems as [id, label] (id)}
            <label class="row"><input type="checkbox" checked={!settings.music[skipKey].includes(id)} onchange={() => toggleSkip(skipKey, id)} /><span class="name" title={label}>{label}</span></label>
          {/each}
        </div>
      </div>
      <small class="muted">Shuffle plays the ticked ones; with none ticked it plays them all. Right-click the note on the left for this list too.</small>
    {/if}

  {:else if tab === "update"}
    <section class="card">
      {@render head("wave", "Update sound", "A short, quiet chime on the first start after Obelisk updates itself.", settings.updateJingle !== "off",
        () => { if (settings.updateJingle === "off") settings.updateJingle = lastJingle; else { lastJingle = settings.updateJingle; settings.updateJingle = "off"; } saveSettings(); })}
      {#if settings.updateJingle !== "off"}
        <div class="card-body">
          {@render volume("Update volume", settings.jingleVolume, (v) => (settings.jingleVolume = v), async () => { await saveSettings(); previewJingle(); })}
          <div class="opt"><span class="opt-label">Sound</span>
            <select class="select" bind:value={settings.updateJingle} onchange={() => { lastJingle = settings.updateJingle; saveSettings(); previewJingle(); }} aria-label="Sound after an update">
              {#each JINGLES as [id, label] (id)}<option value={id}>{label}</option>{/each}
              {#each SOUND_THEMES as [id, label] (id)}<option value={id}>{label} theme</option>{/each}
              <option value="random">A different one each time</option>
            </select>
            <button class="play" data-quiet onclick={previewJingle}>▶ Play</button>
          </div>
        </div>
      {/if}
    </section>

  {:else if tab === "alerts"}
    <section class="card">
      {@render head("bell", "Alert sounds", "Notification sounds for messages, friends and game invites.", settings.cues.on,
        () => { settings.cues.on = !settings.cues.on; saveSettings(); })}
      {#if settings.cues.on}
        <div class="card-body">
          {@render volume("Alert volume", settings.cues.volume, (v) => (settings.cues.volume = v), async () => { await saveSettings(); invoke("cue_preview", { id: "message" }).catch(toastError); })}
          <label class="row-check"><input type="checkbox" bind:checked={settings.cues.dnd} onchange={saveSettings} /> Do Not Disturb
            <small class="muted">Only game invites still make a sound.</small></label>
          <small class="muted">While you play, the small ones are skipped and the rest play quieter.</small>
        </div>
      {/if}
    </section>

    {#if settings.cues.on}
      <div class="list-head">
        <h2>Alert events</h2>
        <input class="search" type="search" placeholder="Search alerts…" bind:value={search} aria-label="Search alerts" />
        <label class="row-check"><input type="checkbox" checked={allOn}
          onchange={(e) => { settings.cues.muted = e.currentTarget.checked ? [] : CUES.map((c) => c[0]); saveSettings(); }} /> Select all</label>
      </div>
      {#each GROUPS as [title, ic, ids] (title)}
        {@const shown = ids.filter(found)}
        {#if shown.length}
          {@const isOpen = search.trim() !== "" || !closed[title]}
          <div class="group">
            <button class="group-head" aria-expanded={isOpen} onclick={() => (closed = { ...closed, [title]: !closed[title] })}>
              {@render icon(ic, 18)}<strong>{title}</strong><span class="count">({ids.length})</span>
              <svg class="chev" class:open={isOpen} viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path d="M9 6l6 6-6 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
            </button>
            {#if isOpen}
              <div class="rows">
                {#each shown as id (id)}
                  <div class="row">
                    <label class="check"><input type="checkbox" checked={!settings.cues.muted.includes(id)} onchange={(e) => setCue(id, e.currentTarget.checked)} /><span class="name">{cueLabel(id)}</span></label>
                    {#if settings.soundTheme === "custom"}<small class="muted pick">{slotSources(id).find((s) => s[0] === soundSource(id))?.[1]}</small>{/if}
                    <button class="play" data-quiet onclick={() => playSlot(id)}>▶ Play</button>
                    <div class="more">
                      <button class="dots" aria-label="More for {cueLabel(id)}" aria-expanded={menuFor === id} onclick={() => (menuFor = menuFor === id ? "" : id)}>⋯</button>
                      {#if menuFor === id}
                        <div class="pop" role="dialog" aria-label="{cueLabel(id)} sound">
                          <label>Sound for {cueLabel(id)}
                            <select class="select" value={soundSource(id)} onchange={(e) => pickSlot(id, e.currentTarget.value)} aria-label="{cueLabel(id)} sound">
                              {#each slotSources(id) as [sid, name] (sid)}<option value={sid}>{name}</option>{/each}
                            </select></label>
                          <small class="muted">Picking one switches the sound theme to Custom; every other sound stays as it is.</small>
                        </div>
                      {/if}
                    </div>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      {/each}
      {#if !GROUPS.some((g) => g[2].some(found))}<small class="muted">No alerts match “{search}”.</small>{/if}
    {/if}

  {:else}
    <section class="card">
      {@render head("palette", "Sound theme", "One instrument for the update sound, menu, typing and alert sounds. Each theme has a music track of the same name.", null, () => {})}
      <div class="card-body">
        <label class="opt"><span class="opt-label">Theme</span>
          <select class="select" value={settings.soundTheme} onchange={(e) => pickSoundTheme(e.currentTarget.value)} aria-label="Sound theme">
            <option value="classic">Classic</option>
            {#each SOUND_THEMES as [id, label] (id)}<option value={id}>{label}</option>{/each}
            <option value="custom">Custom: pick each sound</option>
          </select></label>
        {#if settings.soundTheme === "custom"}
          <div class="slots">
            {#each SOUND_SLOTS.filter(([slot]) => !CUES.some((c) => c[0] === slot)) as [slot, label] (slot)}
              <span>{label}</span>
              <select class="select" value={soundSource(slot)} aria-label="{label} sound" onchange={(e) => pickSlot(slot, e.currentTarget.value)}>
                {#each slotSources(slot) as [id, name] (id)}<option value={id}>{name}</option>{/each}
              </select>
              <button class="play" data-quiet onclick={() => playSlot(slot)}>▶ Play</button>
            {/each}
          </div>
          <small class="muted">Each alert's sound is in Alerts, under its ⋯ button.</small>
        {/if}
      </div>
    </section>

    <section class="card">
      {@render head("pointer", "Menu sounds", "Little ticks and pops as you point at and press things, and a whoosh in and out of modpacks.", settings.uiSounds.on,
        () => { settings.uiSounds.on = !settings.uiSounds.on; saveSettings(); })}
      {#if settings.uiSounds.on}
        <div class="card-body">
          {@render volume("Menu volume", settings.uiSounds.volume, (v) => (settings.uiSounds.volume = v), () => { saveSettings(); uiSound("click", undefined, true); })}
          {#if settings.soundTheme === "classic"}
            <label class="opt"><span class="opt-label">Style</span>
              <select class="select" bind:value={settings.uiSounds.set} onchange={() => { saveSettings(); uiSound("open", settings.uiSounds.set, true); }} aria-label="Menu sound style">
                {#each UI_SETS as [id, label] (id)}<option value={id}>{label}</option>{/each}
              </select></label>
          {/if}
          <div class="opt"><span class="opt-label">Try</span>
            {#each [["hover", "Point"], ["click", "Press"], ["open", "Open"], ["close", "Close"], ["tab", "Tab"]] as const as [kind, label] (kind)}
              <button class="play" data-quiet onclick={() => uiSound(kind, undefined, true)}>{label}</button>
            {/each}
          </div>
        </div>
      {/if}
    </section>

    <section class="card">
      {@render head("keyboard", "Typing sounds", "A tiny tip for each key you type in a text box; each letter has its own note. Never in password fields.", settings.typingSounds.on,
        () => { settings.typingSounds.on = !settings.typingSounds.on; saveSettings(); })}
      {#if settings.typingSounds.on}
        <div class="card-body">
          {@render volume("Typing volume", settings.typingSounds.volume, (v) => (settings.typingSounds.volume = v), () => { saveSettings(); typeSound("key", "a", true); })}
          <label class="opt"><span class="opt-label">Sound</span>
            <select class="select" bind:value={settings.typingSounds.style} aria-label="Typing sound" onchange={() => { saveSettings(); typeSound("key", "a", true); }}>
              <option value="">Match theme</option>
              {#each TYPE_STYLES as [id, label] (id)}<option value={id}>{label}</option>{/each}
              {#each SOUND_THEMES as [id, label] (id)}<option value={id}>{label}</option>{/each}
            </select></label>
          <input class="try" placeholder="Type here to try them" aria-label="Try typing sounds" />
        </div>
      {/if}
    </section>
  {/if}
</div>

<svelte:window onclick={(e) => { if (menuFor && !(e.target as Element).closest?.(".more")) menuFor = ""; }}
  onkeydown={(e) => { if (e.key === "Escape") menuFor = ""; }} />

<style>
  .sound { display: flex; flex-direction: column; gap: 14px; max-width: 760px; }
  h1 { margin: 0; }
  h2 { font-size: 16px; margin: 0; }
  .muted { color: var(--muted); font-weight: 400; }
  .row-check { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .top small { flex-basis: 100%; padding-left: 24px; }

  .tabs { display: flex; flex-wrap: wrap; gap: 4px; padding: 4px; border: 1px solid var(--line); border-radius: 10px; background: var(--panel); align-self: flex-start; max-width: 100%; }
  .tabs button { display: inline-flex; align-items: center; gap: 7px; padding: 7px 14px; border: 0; border-radius: 7px; background: none; color: var(--muted); font: inherit; }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { background: var(--accent); color: #fff; }

  .card { border: 1px solid var(--line); border-radius: 10px; background: var(--panel); }
  .card-head { display: flex; align-items: center; gap: 14px; padding: 14px 16px; }
  .card-icon { flex: none; color: var(--accent); display: grid; }
  .card-title { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .card-title small { color: var(--muted); }
  .card-body { display: flex; flex-direction: column; gap: 10px; padding: 12px 16px 14px; border-top: 1px solid var(--line); }

  .switch { flex: none; width: 40px; height: 22px; border-radius: 999px; border: 0; padding: 0; position: relative; background: var(--switch-off); transition: background 0.15s; }
  .switch i { position: absolute; top: 3px; left: 3px; width: 16px; height: 16px; border-radius: 50%; background: #fff; transition: transform 0.15s; }
  .switch.on { background: var(--accent); }
  .switch.on i { transform: translateX(18px); }

  .vol, .opt { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .vol-label, .opt-label { width: 110px; flex: none; }
  .vol input { width: 200px; max-width: 45vw; accent-color: var(--accent); }
  .vol.disabled { opacity: 0.6; }
  .pct { width: 3.5em; color: var(--muted); }
  .opt .select { min-width: 200px; max-width: 100%; }
  .path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }

  .seg { display: inline-flex; border: 1px solid var(--line); border-radius: 7px; overflow: hidden; }
  .seg button { border: 0; padding: 5px 12px; background: none; color: var(--muted); font: inherit; }
  .seg button + button { border-left: 1px solid var(--line); }
  .seg button.active { background: var(--accent); color: #fff; }

  .ghost { background: none; border: 1px solid var(--line); border-radius: 6px; padding: 5px 12px; color: inherit; font: inherit; }
  .play { flex: none; background: none; border: 1px solid var(--line); border-radius: 6px; padding: 3px 10px; color: inherit; font: inherit; font-size: 13px; white-space: nowrap; }
  .play:hover, .ghost:hover, .dots:hover { border-color: var(--accent); }

  .list-head { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-top: 4px; }
  .list-head h2 { flex: 1; }
  .search { background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 5px 10px; font: inherit; width: 200px; max-width: 100%; }

  .group { border: 1px solid var(--line); border-radius: 10px; background: var(--panel); }
  .group-head { display: flex; align-items: center; gap: 10px; width: 100%; padding: 10px 14px; border: 0; background: none; color: inherit; font: inherit; text-align: left; }
  .count { color: var(--muted); flex: 1; }
  .chev { transition: transform 0.15s; color: var(--muted); }
  .chev.open { transform: rotate(90deg); }
  .rows { display: flex; flex-direction: column; border-top: 1px solid var(--line); padding: 4px 0; }
  .rows.cols { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); border-top: 0; }
  .row { display: flex; align-items: center; gap: 10px; padding: 4px 14px 4px 18px; min-height: 32px; }
  .row:hover { background: var(--row-line); }
  .check { display: flex; align-items: center; gap: 10px; flex: 1; min-width: 0; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pick { font-size: 12px; }
  input[type="checkbox"] { accent-color: var(--accent); }

  .more { position: relative; }
  .dots { background: none; border: 1px solid transparent; border-radius: 6px; padding: 0 8px; color: inherit; font-size: 18px; line-height: 24px; }
  .pop { position: absolute; right: 0; top: calc(100% + 4px); z-index: 20; width: 240px; display: flex; flex-direction: column; gap: 8px; padding: 12px;
    background: var(--panel); border: 1px solid var(--line); border-radius: 8px; box-shadow: 0 6px 24px rgb(0 0 0 / 0.35); }
  .pop label { display: flex; flex-direction: column; gap: 6px; }

  .slots { display: grid; grid-template-columns: max-content minmax(0, 220px) auto; gap: 6px 12px; align-items: center; justify-content: start; }
  .try { background: var(--bg); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 6px 10px; font: inherit; width: 260px; max-width: 100%; }

  @media (max-width: 640px) {
    .vol-label, .opt-label { width: auto; }
    .tabs button span { display: none; }
  }
</style>
