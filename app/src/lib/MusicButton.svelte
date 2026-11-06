<script lang="ts">
  // The note on the left rail while background music is on: click to pause or resume (player.svelte.ts plays it).
  import { onMount } from "svelte";
  import { settings } from "./settings.svelte";
  import { music, sync, watchWindow } from "./player.svelte";

  onMount(watchWindow);
  $effect(() => {
    // Any change to the music settings (or pausing) brings the player in line.
    void [settings.music.on, settings.music.source, settings.music.track, settings.music.folder, settings.music.focusOnly, settings.music.volume, music.paused];
    sync();
  });
</script>

{#if settings.music.on}
  <button class="note" class:paused={music.paused} aria-label={music.paused ? "Resume music" : "Pause music"}
    title={music.error || (music.paused ? "Music paused: click to resume" : `${music.title ? `Playing ${music.title}. ` : ""}Click to pause`)}
    onclick={() => (music.paused = !music.paused)}>
    <svg viewBox="0 0 24 24" width="20" height="20"><path d="M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round" /></svg>
    {#if music.paused}<i></i>{/if}
  </button>
{/if}

<style>
  .note { position: relative; width: 44px; height: 44px; display: grid; place-items: center; border: 0; border-radius: 10px; background: none; color: var(--text); }
  .note.paused { color: var(--muted); }
  .note:hover { background: var(--panel); }
  /* A slash through the note while paused. */
  .note i { position: absolute; width: 26px; height: 2px; background: currentColor; transform: rotate(-45deg); border-radius: 2px; }
</style>
