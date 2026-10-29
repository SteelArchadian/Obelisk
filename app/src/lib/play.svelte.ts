import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { app, toast, toastError } from "./state.svelte";
import type { Instance } from "./api";

/** Profiles whose game is on its way: Play buttons show "Starting…" until the game window opens. */
export const starting = $state<Record<string, boolean>>({});
/** Profiles whose game is running: Play becomes Stop. */
export const running = $state<Record<string, boolean>>({});
/** Prism's download/prepare progress while starting, shown on the profile tile (pct null = unknown). */
export const progress = $state<Record<string, { pct: number | null; text: string }>>({});

let listening = false;
function listenOnce() {
  if (listening) return;
  listening = true;
  listen<{ id: string; up: boolean }>("game-started", (e) => {
    const { id, up } = e.payload;
    if (up) running[id] = true;
    delete progress[id];
    if (!starting[id]) return;
    delete starting[id];
    if (!up) toastError("The game didn't start.");
  }).catch(() => (listening = false));
  // Prism's console stays shut on a crash (quiet launch); say so here and point at the diagnosis.
  listen<{ id: string; file: string }>("game-crashed", (e) => {
    const name = app.instances.find((i) => i.id === e.payload.id)?.name ?? e.payload.id;
    toastError(`${name} crashed. Open it and look at Diagnosis to see why.`);
  }).catch(() => {});
  listen<{ id: string }>("game-exited", (e) => { delete running[e.payload.id]; delete starting[e.payload.id]; delete progress[e.payload.id]; }).catch(() => {});
  listen<{ id: string; pct: number | null; text: string }>("launch-progress", (e) => {
    const { id, pct, text } = e.payload;
    if (starting[id]) progress[id] = { pct, text };
  }).catch(() => {});
}

/** Pick up games already running (e.g. started before Obelisk opened). */
export async function refreshRunning() {
  listenOnce();
  const ids = await invoke<string[]>("running_games").catch(() => [] as string[]);
  for (const k of Object.keys(running)) if (!ids.includes(k)) delete running[k];
  for (const id of ids) running[id] = true;
}

/** Start a profile's game through Prism (quietly, unless turned off in Settings). */
export async function play(inst: Instance) {
  if (starting[inst.id] || running[inst.id]) return;
  listenOnce();
  starting[inst.id] = true;
  try {
    const javaNote = await invoke<string | null>("launch", { instanceId: inst.id });
    if (javaNote) toast(javaNote);
    toast(`Starting ${inst.name}… big packs can take a minute or two.`);
  } catch (e) {
    const cancelled = !starting[inst.id];
    delete starting[inst.id];
    if (!cancelled) toastError(e);
  }
}

/** Stop a game that's still starting (a misclick): nothing is lost, no world is open yet. */
export async function cancel(inst: Instance) {
  if (!starting[inst.id]) return;
  delete starting[inst.id];
  delete progress[inst.id];
  try {
    await invoke("cancel_launch", { instanceId: inst.id });
    toast(`Aborted starting ${inst.name}.`);
  } catch (e) {
    toastError(e);
  }
}

/** Force-close a frozen game. Unsaved progress since the last autosave is lost. */
export async function kill(inst: Instance) {
  try {
    await invoke("kill_game", { instanceId: inst.id });
    delete running[inst.id];
    delete starting[inst.id];
    toast(`${inst.name} was closed.`);
  } catch (e) {
    toastError(e);
    refreshRunning();
  }
}
