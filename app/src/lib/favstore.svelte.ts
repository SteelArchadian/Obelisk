import { invoke } from "@tauri-apps/api/core";
import type { Hit } from "./api";

// Projects saved for later with the heart on their page, shared by every pack, so a mod can wait until it's talked
// over (say with a friend on a shared pack) and then be installed from any pack's Favorites tab. Un-hearting moves it
// to Removed for a day, in case that was a slip.
export type Fav = { hit: Hit; addedMs: number; removedMs?: number };

const DAY = 24 * 60 * 60 * 1000;

export const favorites = $state<{ saved: Fav[]; removed: Fav[] }>({ saved: [], removed: [] });

const prune = () => {
  const cutoff = Date.now() - DAY;
  favorites.removed = favorites.removed.filter((f) => (f.removedMs ?? 0) > cutoff);
};

export async function loadFavorites() {
  const saved = await invoke<Partial<typeof favorites>>("get_favorites").catch((): Partial<typeof favorites> => ({}));
  favorites.saved = saved.saved ?? [];
  favorites.removed = saved.removed ?? [];
  prune();
}

async function persist() {
  prune();
  await invoke("set_favorites", { favorites: $state.snapshot(favorites) }).catch(() => {});
}

export const isFavorite = (id: string) => favorites.saved.some((f) => f.hit.project_id === id);

export function toggleFavorite(hit: Hit) {
  const id = hit.project_id;
  const i = favorites.saved.findIndex((f) => f.hit.project_id === id);
  if (i >= 0) {
    const [f] = favorites.saved.splice(i, 1);
    favorites.removed = [{ ...f, removedMs: Date.now() }, ...favorites.removed.filter((r) => r.hit.project_id !== id)];
  } else {
    const back = favorites.removed.find((r) => r.hit.project_id === id);
    favorites.removed = favorites.removed.filter((r) => r.hit.project_id !== id);
    favorites.saved = [{ hit: $state.snapshot(hit) as Hit, addedMs: back?.addedMs ?? Date.now() }, ...favorites.saved];
  }
  persist();
}

/** Removed ones, newest removal first; anything older than a day has already been dropped. */
export const removedNewestFirst = () => [...favorites.removed].sort((a, b) => (b.removedMs ?? 0) - (a.removedMs ?? 0));
