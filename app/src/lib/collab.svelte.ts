import { invoke } from "@tauri-apps/api/core";
import type { Hit } from "./api";

// A collaborative pack as its page shows it: who's in it, who owns it, and everyone's wish lists (the star on a
// project page adds to mine; friends' lists arrive with the regular sync).
export type Member = { id: string; name: string; lastSeenMs: number };
export type Wish = { hit: Hit; addedMs: number };
export type WishList = { name: string; updatedMs: number; items: Wish[] };
export type Collab = { shared: boolean; me?: string; owner?: string; members?: Member[]; wishes?: Record<string, WishList>; ended?: { byName: string; atMs: number } | null };

export const collab = $state<Record<string, Collab>>({});

export async function loadCollab(instId: string) {
  try {
    collab[instId] = await invoke<Collab>("share_info", { instanceId: instId });
  } catch {
    /* not reachable right now: keep what we had */
  }
}

export const myWishes = (instId: string): Wish[] => {
  const c = collab[instId];
  return (c?.me && c.wishes?.[c.me]?.items) || [];
};

export const isWished = (instId: string, projectId: string) => myWishes(instId).some((w) => w.hit.project_id === projectId);

export async function toggleWish(instId: string, hit: Hit) {
  const c = collab[instId];
  if (!c?.me) return;
  const mine = myWishes(instId);
  const items = mine.some((w) => w.hit.project_id === hit.project_id)
    ? mine.filter((w) => w.hit.project_id !== hit.project_id)
    : [{ hit: $state.snapshot(hit) as Hit, addedMs: Date.now() }, ...mine];
  const name = c.members?.find((m) => m.id === c.me)?.name ?? "";
  c.wishes = { ...(c.wishes ?? {}), [c.me]: { name, updatedMs: Date.now(), items } };
  await invoke("share_set_wishes", { instanceId: instId, items }).catch(() => {});
}
