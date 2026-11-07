<script lang="ts">
  // The note on the left rail while background music is on: click to pause or resume (player.svelte.ts plays it).
  // Right-click opens the Shuffle list: tick the tracks (or own files) it plays, or skip to the next one.
  // Always mounted, so it also runs the menu sounds (uisound.svelte.ts).
  import { onMount } from "svelte";
  import { settings, saveSettings, TRACKS } from "./settings.svelte";
  import { music, next, sync, watchWindow } from "./player.svelte";
  import { navSound, watchUi } from "./uisound.svelte";

  let menu = $state(false);
  let at = $state({ x: 0, y: 0 });
  const mine = $derived(settings.music.source === "mine");
  const items = $derived<[string, string][]>(mine ? music.files.map((f) => [f, f]) : TRACKS);
  function toggle(id: string) {
    const key = mine ? "skipFiles" : "skip";
    const list = settings.music[key];
    settings.music[key] = list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
    if (!mine) settings.music.track = "all";
    saveSettings();
  }

  $effect(() => {
    if (!menu) return;
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") menu = false; };
    addEventListener("keydown", esc);
    return () => removeEventListener("keydown", esc);
  });

  onMount(() => {
    const window = watchWindow();
    const ui = watchUi();
    return () => { window(); ui(); };
  });
  $effect(() => {
    // Any change to the music settings (or pausing) brings the player in line.
    void [settings.music.on, settings.music.source, settings.music.track, settings.music.folder, JSON.stringify(settings.music.skip), JSON.stringify(settings.music.skipFiles), settings.muteInactive, settings.music.volume, music.paused];
    sync();
  });
  $effect(navSound);
</script>

{#if settings.music.on}
  <button class="note" class:paused={music.paused} aria-label={music.paused ? "Resume music" : "Pause music"}
    title={music.error || (music.paused ? "Music paused: click to resume" : `${music.title ? `Playing ${music.title}. ` : ""}Click to pause`)}
    onclick={() => (music.paused = !music.paused)} oncontextmenu={(e) => { e.preventDefault(); const r = e.currentTarget.getBoundingClientRect(); at = { x: r.right + 8, y: innerHeight - r.bottom }; menu = !menu; }}>
    <svg viewBox="0 0 24 24" width="20" height="20"><path d="M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" /></svg>
    {#if music.paused}<i></i>{/if}
  </button>
  {#if menu}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events (Escape closes it too) -->
    <div class="shade" onclick={() => (menu = false)} oncontextmenu={(e) => { e.preventDefault(); menu = false; }}></div>
    <div class="menu" role="dialog" aria-label="Shuffle" style="left: {at.x}px; bottom: {at.y}px;">
      <strong>Shuffle</strong>
      <div class="list">
        {#each items as [id, label] (id)}
          <label><input type="checkbox" checked={(mine || settings.music.track === "all") && !settings.music[mine ? "skipFiles" : "skip"].includes(id)} onchange={() => toggle(id)} /> {label}</label>
        {:else}<small>No music files found yet.</small>{/each}
      </div>
      <button onclick={() => { music.paused = false; next(); }}>Next track</button>
    </div>
  {/if}
{/if}

<style>
  .note { position: relative; width: 44px; height: 44px; display: grid; place-items: center; border: 0; border-radius: 10px; background: none; color: var(--text); }
  .note.paused { color: var(--muted); }
  .note:hover { background: var(--panel); }
  /* A slash through the note while paused. */
  .shade { position: fixed; inset: 0; z-index: 50; }
  .menu { position: fixed; z-index: 51; width: 240px; max-height: 360px; display: flex; flex-direction: column; gap: 6px;
    padding: 10px; border-radius: 10px; background: var(--panel); color: var(--text); box-shadow: 0 6px 24px rgb(0 0 0 / 0.35); }
  .menu .list { overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }
  .menu label { display: flex; gap: 6px; align-items: center; font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .menu button { border: 0; border-radius: 8px; padding: 6px; background: var(--bg, rgb(0 0 0 / 0.2)); color: var(--text); }
  .note i { position: absolute; width: 26px; height: 2px; background: currentColor; transform: rotate(-45deg); border-radius: 2px; }
</style>
