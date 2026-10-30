// Signed-in site accounts: Modrinth (a personal access token kept in Windows Credential Manager) and CurseForge
// (signed in on curseforge.com's own page in a small window; its session stays in Obelisk's web view) for following
// projects, comments and notifications.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast, toastError } from "./state.svelte";

export type MrAccount = { id: string; username: string; avatar_url?: string };
export type MrNotification = { id: string; title: string; text: string; link: string; read: boolean; created: string; type?: string };

export const mr = $state({
  account: null as MrAccount | null,
  follows: new Set<string>(),
  notes: [] as MrNotification[],
  loaded: false,
});

export type CfAccount = { id: number; username: string; displayName: string; avatarUrl?: string };
/** A CurseForge notification: the site's own ("site-N": approvals, status changes, comments, replies, messages) or a reply Obelisk found. */
export type CfNotification = {
  id: string; title: string; text: string; label?: string; kind?: string; projectId?: number; tab?: "comments" | "versions" | null;
  url?: string | null; read: boolean; created: number | string;
};

export const cf = $state({
  account: null as CfAccount | null,
  favorites: new Set<number>(),
  notes: [] as CfNotification[],
  signingIn: false,
});

export const cfUnread = () => cf.notes.filter((n) => !n.read).length;
export const unread = () => mr.notes.filter((n) => !n.read).length + cfUnread();

let cfListening = false;

export async function loadCf() {
  if (!cfListening) {
    cfListening = true;
    listen("cf-account", () => { cf.signingIn = false; loadCf(); });
    listen("cf-notes", () => loadCfNotes());
    listen("cf-signin-ended", () => { cf.signingIn = false; });
  }
  try {
    cf.account = await invoke<CfAccount | null>("cf_account");
    if (cf.account) {
      try { cf.favorites = new Set(await invoke<number[]>("cf_favorites")); } catch { /* favorites are extra */ }
    }
  } catch { /* offline: shown as signed out */ }
  await loadCfNotes();
}

export async function loadCfNotes() {
  try { cf.notes = await invoke<CfNotification[]>("cf_notifications"); } catch { /* keep what we had */ }
}

/** Opens curseforge.com's sign-in page in a small window; it closes by itself once signed in. */
export async function cfSignIn() {
  try { cf.signingIn = true; await invoke("cf_signin"); } catch (e) { cf.signingIn = false; toastError(e); }
}

export async function cfSignOut() {
  try {
    await invoke("cf_signout");
    cf.account = null;
    cf.favorites = new Set();
  } catch (e) { toastError(e); }
}

export async function toggleCfFavorite(projectId: number) {
  const on = !cf.favorites.has(projectId);
  try {
    await invoke("cf_favorite", { projectId, on });
    const s = new Set(cf.favorites);
    on ? s.add(projectId) : s.delete(projectId);
    cf.favorites = s;
  } catch (e) { toastError(e); }
}

export async function cfMarkRead(ids: string[]) {
  if (!ids.length) return;
  cf.notes = cf.notes.map((n) => (ids.includes(n.id) ? { ...n, read: true } : n));
  try { await invoke("cf_notifications_read", { ids }); } catch (e) { toastError(e); }
}

export async function loadAccount() {
  try {
    mr.account = await invoke<MrAccount | null>("mr_account");
    if (mr.account) {
      mr.follows = new Set(await invoke<string[]>("mr_follows"));
      await loadNotes();
    }
  } catch { /* offline or token revoked: shown as signed out */ }
  mr.loaded = true;
}

export async function loadNotes() {
  if (!mr.account) return;
  try {
    const n = await invoke<MrNotification[]>("mr_notifications");
    mr.notes = n.sort((a, b) => b.created.localeCompare(a.created));
  } catch { /* keep what we had */ }
}

export async function signIn(token: string) {
  mr.account = await invoke<MrAccount | null>("mr_token_set", { token });
  await loadAccount();
}

export async function signOut() {
  await invoke("mr_token_set", { token: "" });
  mr.account = null;
  mr.follows = new Set();
  mr.notes = [];
}

export async function toggleFollow(projectId: string) {
  const on = !mr.follows.has(projectId);
  try {
    await invoke("mr_follow", { id: projectId, on });
    const s = new Set(mr.follows);
    on ? s.add(projectId) : s.delete(projectId);
    mr.follows = s;
  } catch (e) { toastError(e); }
}

export async function markRead(ids: string[]) {
  if (!ids.length) return;
  try {
    await invoke("mr_notifications_read", { ids });
    mr.notes = mr.notes.map((n) => (ids.includes(n.id) ? { ...n, read: true } : n));
  } catch (e) { toastError(e); }
}

/** Open a web link in the default browser. Authors sometimes leave out the https://, which the opener refuses. */
export async function openLink(url: string | undefined | null) {
  if (!url) return;
  let u = url.trim();
  if (!/^[a-z][a-z0-9+.-]*:/i.test(u)) u = `https://${u.replace(/^\/+/, "")}`;
  if (!/^https?:/i.test(u)) { toast("That link isn't a web address, so Obelisk won't open it."); return; }
  try { await openUrl(u); } catch (e) { toastError(`Couldn't open the link: ${e}`); }
}
