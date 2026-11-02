import { api, type FileInfo, type Instance } from "./api";

/** Re-check a profile in the background when its last check is older than this. */
const STALE = 12 * 60 * 60 * 1000;
type Cached = { at: number; infos: FileInfo[] };
const memory = new Map<string, Cached>();
const inflight = new Map<string, Promise<FileInfo[]>>();

/** Bumped whenever new results land, so open pages pick them up. */
export const updateState = $state({ tick: 0, running: 0 });

const keyOf = (id: string) => `obelisk.updates.v4.${id}`;

/**
 * Files being updated right now, per profile ("kind/fileName"). Kept here, not in the page, so leaving a
 * profile mid-Update all doesn't lose the spinner or the countdown.
 */
export const updating = $state<Record<string, string[]>>({});

export function setUpdating(id: string, key: string, on: boolean) {
  const rest = (updating[id] ?? []).filter((k) => k !== key);
  updating[id] = on ? [...rest, key] : rest;
}

/** One file is up to date: drop its update from the stored results straight away (the re-check fills in the rest). */
export function markUpdated(id: string, key: string) {
  const c = cached(id);
  if (!c) return;
  const entry = { at: c.at, infos: c.infos.map((f) => `${f.kind}/${f.fileName}` === key ? { ...f, update: null } : f) };
  memory.set(id, entry);
  try { localStorage.setItem(keyOf(id), JSON.stringify(entry)); } catch { /* ignore */ }
  updateState.tick++;
}

/** Last results for a profile (any age), without touching the network. */
export function cached(id: string): Cached | null {
  if (memory.has(id)) return memory.get(id)!;
  try {
    const c = JSON.parse(localStorage.getItem(keyOf(id)) ?? "null");
    if (c) memory.set(id, c);
    return c;
  } catch {
    return null;
  }
}

/** Ask the sites now (one request at a time per profile). */
export function recheck(id: string): Promise<FileInfo[]> {
  if (inflight.has(id)) return inflight.get(id)!;
  updateState.running++;
  const p = api.checkUpdates(id).then((infos) => {
    const entry = { at: Date.now(), infos };
    memory.set(id, entry);
    try { localStorage.setItem(keyOf(id), JSON.stringify(entry)); } catch { /* ignore */ }
    updateState.tick++;
    return infos;
  }).finally(() => {
    inflight.delete(id);
    updateState.running--;
  });
  inflight.set(id, p);
  return p;
}

/** Cached results if there are any; only a profile never checked before goes to the network. */
export function updatesFor(id: string): Promise<FileInfo[]> {
  const c = cached(id);
  return c ? Promise.resolve(c.infos) : recheck(id);
}

/** Quietly refresh every profile whose results are older than 12 hours (on launch, then on a timer). */
export async function backgroundCheck(instances: Instance[]) {
  for (const inst of instances) {
    const c = cached(inst.id);
    if (c && Date.now() - c.at < STALE) continue;
    await recheck(inst.id).catch(() => {});
  }
}
