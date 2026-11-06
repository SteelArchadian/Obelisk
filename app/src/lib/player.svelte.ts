// Background music (Settings → Sound, on by default): Obelisk's own original tracks (music.rs renders them) or the
// user's own files from a folder they pick. Plays only while the window is active (unless "mute when Obelisk isn't the
// active window" is off), and fades in and out instead of cutting. MusicButton.svelte (the note on the left rail) pauses it.
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { settings, TRACKS, soundGain } from "./settings.svelte";

export const music = $state({ paused: false, title: "", error: "" });

let audio: HTMLAudioElement | null = null;
let list: { url: string; title: string }[] = [];
let at = 0;
let loaded = "";
let focused = typeof document !== "undefined" && document.hasFocus();
let shown = true;
let fader: ReturnType<typeof setInterval> | undefined;

const wanted = () => settings.music.on && !music.paused && shown && (!settings.muteInactive || focused);
const name = (path: string) => path.split(/[\\/]/).pop()!.replace(/\.[^.]+$/, "");

/** Follows the window: focus for muting while inactive, and hidden in the tray (closed) means not open. */
export function watchWindow() {
  const win = getCurrentWindow();
  const a = win.onFocusChanged(({ payload }) => { focused = payload; if (payload) shown = true; sync(); });
  const b = win.onCloseRequested(() => { if (settings.closeToTray) { shown = false; sync(); } });
  return () => { a.then((u) => u()); b.then((u) => u()); };
}

/** Brings the player in line with the settings and the window: loads what to play, then fades in or out. */
export async function sync() {
  const m = settings.music;
  const key = JSON.stringify([m.source, m.track, m.folder]);
  if (wanted() && key !== loaded) {
    loaded = key;
    stop();
    try {
      list = m.source === "mine"
        ? shuffle((await invoke<string[]>("music_files", { folder: m.folder })).map((p) => ({ url: convertFileSrc(p), title: name(p) })))
        : await Promise.all((m.track === "all" ? shuffle(TRACKS.map((t) => t[0])) : [m.track]).map(async (id) =>
            ({ url: convertFileSrc(await invoke<string>("music_track", { id })), title: TRACKS.find((t) => t[0] === id)?.[1] ?? id })));
      music.error = m.source === "mine" && !list.length ? "No music files in that folder (mp3, ogg, opus, wav, flac or m4a)." : "";
    } catch (e) {
      list = [];
      music.error = String(e);
    }
    at = 0;
  }
  if (!list.length) return;
  if (!audio) start();
  fade(wanted() ? soundGain(settings.music.volume) : 0);
}

function start() {
  const track = list[at % list.length];
  audio = new Audio(track.url);
  audio.loop = list.length === 1;
  audio.volume = 0;
  audio.onended = () => { at++; stop(); if (wanted()) { start(); fade(soundGain(settings.music.volume)); } };
  audio.onerror = () => { music.error = `Couldn't play ${track.title}.`; };
  music.title = track.title;
}

function stop() {
  clearInterval(fader);
  audio?.pause();
  if (audio) audio.onended = null;
  audio = null;
  music.title = "";
}

/** Glides the volume to `to` over about half a second; at 0 the music pauses (so it resumes where it was). */
function fade(to: number) {
  clearInterval(fader);
  const a = audio!;
  // If the webview won't start sound before the user has touched the window, start on the first click instead.
  if (to > 0 && a.paused) a.play().catch((e) => {
    if (e?.name === "NotAllowedError") window.addEventListener("pointerdown", () => sync(), { once: true });
    else music.error = String(e);
  });
  // About 20 steps whatever the level, so quiet settings fade as smoothly as loud ones.
  const step = Math.max(to, a.volume, 0.01) / 20;
  fader = setInterval(() => {
    const v = a.volume + Math.sign(to - a.volume) * step;
    a.volume = Math.abs(to - a.volume) <= step ? to : Math.min(1, Math.max(0, v));
    if (a.volume !== to) return;
    clearInterval(fader);
    if (to === 0) a.pause();
  }, 25);
}

/** Skips to the next file or track. */
export function next() {
  if (list.length < 2) return;
  at++;
  stop();
  start();
  sync();
}

function shuffle<T>(a: T[]): T[] {
  for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; }
  return a;
}
