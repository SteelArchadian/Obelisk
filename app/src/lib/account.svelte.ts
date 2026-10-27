// Signed-in site accounts: Modrinth (a personal access token kept in Windows Credential Manager) for following
// projects and notifications. CurseForge has no sign-in for other apps, so it has neither.
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "./state.svelte";

export type MrAccount = { id: string; username: string; avatar_url?: string };
export type MrNotification = { id: string; title: string; text: string; link: string; read: boolean; created: string; type?: string };

export const mr = $state({
  account: null as MrAccount | null,
  follows: new Set<string>(),
  notes: [] as MrNotification[],
  loaded: false,
});

export const unread = () => mr.notes.filter((n) => !n.read).length;

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
  } catch (e) { toast(String(e)); }
}

export async function markRead(ids: string[]) {
  if (!ids.length) return;
  try {
    await invoke("mr_notifications_read", { ids });
    mr.notes = mr.notes.map((n) => (ids.includes(n.id) ? { ...n, read: true } : n));
  } catch (e) { toast(String(e)); }
}

/** Open a web link in the default browser. Authors sometimes leave out the https://, which the opener refuses. */
export async function openLink(url: string | undefined | null) {
  if (!url) return;
  let u = url.trim();
  if (!/^[a-z][a-z0-9+.-]*:/i.test(u)) u = `https://${u.replace(/^\/+/, "")}`;
  if (!/^https?:/i.test(u)) { toast("That link isn't a web address, so Obelisk won't open it."); return; }
  try { await openUrl(u); } catch (e) { toast(`Couldn't open the link: ${e}`); }
}
