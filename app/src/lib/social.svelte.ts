// Friends, messages and invites (social/ in the Rust side). Off unless a provider is on; "mock" fills it with
// made-up friends and a few scripted events, for building the panel and trying the sounds.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "./state.svelte";

export type PackRef = { name: string; mc: string; loader: string; modsHash?: string; source?: string };
export type Presence = { state: "Offline" } | { state: "Online" } | { state: "Playing"; pack?: PackRef; world?: string; joinable: boolean };
export type Friend = { id: string; name: string; uuid?: string; avatar?: string; presence: Presence; since?: number };
export type Message = { id: string; conversation: string; from: string; text: string; sent: number };
export type Conversation = { id: string; title: string; members: string[]; unread: number; last?: Message };
export type Invite = { id: string; from: string; pack?: PackRef; target: string; expires?: number };
export type SocialEvent =
  | { kind: "friend_request"; from: Friend } | { kind: "friend_accepted"; friend: Friend }
  | { kind: "friend_online" | "friend_offline" | "joined_world"; id: string } | { kind: "presence"; id: string; presence: Presence }
  | { kind: "message" | "message_sent" | "mention"; message: Message } | { kind: "invite"; invite: Invite }
  | { kind: "invite_accepted" | "invite_declined"; invite: string; by: string }
  | { kind: "connected" | "reconnected" } | { kind: "disconnected"; why: string } | { kind: "error"; text: string };

export const social = $state({
  provider: null as string | null,
  friends: [] as Friend[],
  requests: [] as Friend[],
  conversations: [] as Conversation[],
  invites: [] as Invite[],
});

const name = (id: string) => social.friends.find((f) => f.id === id)?.name ?? "A friend";

async function refresh() {
  if (!social.provider) return;
  const r = await invoke<{ friends: Friend[]; requests: Friend[] }>("social_friends").catch(() => null);
  if (r) { social.friends = r.friends; social.requests = r.requests; }
  social.conversations = await invoke<Conversation[]>("social_conversations").catch(() => social.conversations);
}

function on(e: SocialEvent) {
  switch (e.kind) {
    case "presence": { const f = social.friends.find((x) => x.id === e.id); if (f) f.presence = e.presence; break; }
    case "friend_online": { const f = social.friends.find((x) => x.id === e.id); if (f && f.presence.state === "Offline") f.presence = { state: "Online" }; break; }
    case "friend_offline": { const f = social.friends.find((x) => x.id === e.id); if (f) f.presence = { state: "Offline" }; break; }
    case "friend_request": social.requests = [...social.requests, e.from]; toast(`${e.from.name} wants to be friends.`); break;
    case "friend_accepted": refresh(); toast(`${e.friend.name} is now your friend.`); break;
    case "message": case "mention": {
      const c = social.conversations.find((x) => x.id === e.message.conversation);
      if (c) { c.unread += 1; c.last = e.message; } else refresh();
      toast(`${name(e.message.from)}: ${e.message.text}`);
      break;
    }
    case "invite": social.invites = [...social.invites, e.invite]; toast(`${name(e.invite.from)} invited you to play.`); break;
    case "invite_accepted": toast(`${name(e.by)} is joining you.`); break;
    case "disconnected": toast(`Friends are offline for now (${e.why}).`); break;
    case "error": toast(e.text); break;
  }
}

let started = false;
/** Called once at start-up: listens for events and picks up a provider that's already on. */
export async function startSocial() {
  if (started) return;
  started = true;
  listen<SocialEvent>("social-event", ({ payload }) => on(payload));
  social.provider = await invoke<string | null>("social_status").catch(() => null);
  refresh();
}

/** "off", "mock", later "essential" / "obelisk". */
export async function useSocial(provider: string) {
  await invoke("social_use", { provider });
  social.provider = provider === "off" ? null : provider;
  social.friends = []; social.requests = []; social.conversations = []; social.invites = [];
  refresh();
}

export const sendMessage = (conversation: string, text: string) => invoke<Message>("social_send", { conversation, text });
export const answerInvite = async (id: string, accept: boolean) => {
  await invoke("social_answer_invite", { id, accept });
  social.invites = social.invites.filter((i) => i.id !== id);
};
