<script lang="ts">
  // Collaborate: play a pack with friends, peer to peer: a short code to invite them (pasted into Import), who's in,
  // my changes to send, and their changes waiting for review. Applying takes a safety backup first, so it can
  // always be undone. Unticking "Collaboration on" stops it. The owner (who made it collaborative) can remove people;
  // History shows who changed what.
  import { invoke } from "@tauri-apps/api/core";
  import { app, toast, toastError } from "./state.svelte";
  import { syncInstances } from "./install";
  import type { Instance } from "./api";

  type Change = { kind: "added" | "removed" | "changed"; area: string; path: string; label: string };
  type Member = { id: string; name: string; lastSeenMs: number };
  type Changeset = { id: string; author: string; authorName: string; atMs: number; label: string; changes: Change[] };
  type Status = { shared: boolean; me: string; owner: string; share: { packId: string; name: string; members: Member[]; autoApply: boolean; autoSend?: boolean } | null; incoming: Changeset[]; local: Change[]; held?: Held[] };
  type Held = { path: string; size: number; from: string; reasons: string[] };

  let { inst }: { inst: Instance } = $props();
  let st = $state<Status | null>(null);
  let code = $state("");
  let expiresMs = $state(0);
  let now = $state(Date.now());
  $effect(() => { const t = setInterval(() => (now = Date.now()), 1000); return () => clearInterval(t); });
  const left = $derived(Math.max(0, expiresMs - now));
  const clock = $derived(`${Math.floor(left / 60000)}:${String(Math.floor((left % 60000) / 1000)).padStart(2, "0")}`);
  let label = $state("");
  let busy = $state("");
  let open = $state<string | null>(null);
  let confirmStop = $state(false);
  let confirmRemove = $state<Member | null>(null);
  let history = $state<Changeset[] | null>(null);
  let historyOpen = $state(false);
  const isOwner = $derived(!!st && st.owner === st.me);

  const load = () => invoke<Status>("share_status", { instanceId: inst.id }).then((s) => (st = s)).catch((e) => toastError(e));
  $effect(() => { void inst.id; load(); });

  const close = () => (app.sharePack = null);
  async function run<T>(what: string, fn: () => Promise<T>): Promise<T | undefined> {
    busy = what;
    try { return await fn(); } catch (e) { toastError(e); } finally { busy = ""; }
  }

  const getCode = async () => {
    const c = await invoke<{ code: string; expiresMs: number }>("collab_code", { instanceId: inst.id });
    code = c.code;
    expiresMs = c.expiresMs;
  };
  const start = () => run("start", async () => { await getCode(); await load(); });
  const invite = () => run("invite", getCode);
  let copied = $state(false);
  const copy = () => navigator.clipboard.writeText(code).then(() => {
    copied = true;
    setTimeout(() => (copied = false), 2000);
    toast("Code copied. Your friend pastes it into Import.");
  });
  const send = () => run("send", async () => {
    const sent = await invoke<boolean>("share_publish", { instanceId: inst.id, label });
    label = "";
    toast(sent ? "Your changes are ready for your friends. They'll get them next time you're both online." : "Nothing new to send.");
    await load();
  });
  const sync = () => run("sync", async () => {
    const n = await invoke<number>("share_sync");
    toast(n ? `Got ${n} new change${n === 1 ? "" : "s"} from friends.` : "Up to date with everyone who's online.");
    await load();
  });
  const apply = (ids: string[]) => run("apply", async () => {
    const r = await invoke<{ applied: number; conflicts: string[]; blocked: string[] }>("share_apply", { instanceId: inst.id, ids });
    toast(r.blocked.length
      ? `Applied, except ${r.blocked.length} blocked file${r.blocked.length === 1 ? "" : "s"}: see Blocked files.`
      : r.conflicts.length
      ? `Applied. ${r.conflicts.length} file${r.conflicts.length === 1 ? "" : "s"} you'd also changed now use their version; yours is in Backups.`
      : `Applied ${r.applied} change${r.applied === 1 ? "" : "s"}. Undo anytime from Backups.`);
    await load();
    window.dispatchEvent(new Event("share-changed"));
  });
  const skip = (id: string) => run("skip", async () => { await invoke("share_skip", { instanceId: inst.id, id }); await load(); window.dispatchEvent(new Event("share-changed")); });
  // Blocked files are reviewed in their own window (HeldReview), where only Windows' box can let one in.
  const review = () => window.dispatchEvent(new CustomEvent("held-review", { detail: inst.id }));
  $effect(() => { window.addEventListener("held-changed", load); return () => window.removeEventListener("held-changed", load); });
  const setAuto = (on: boolean) => run("auto", async () => { await invoke("share_set_auto", { instanceId: inst.id, on }); await load(); });
  const setAutoSend = (on: boolean) => run("autoSend", async () => { await invoke("share_set_auto_send", { instanceId: inst.id, on }); await load(); });
  const remove = (m: Member) => run("remove", async () => {
    await invoke("share_remove", { instanceId: inst.id, member: m.id });
    confirmRemove = null;
    toast(`Removed ${m.name || "them"}. They'll be told next time their Obelisk connects, and the pack has a new key, so old invite codes no longer work.`);
    await load();
    window.dispatchEvent(new Event("share-changed"));
  });
  const showHistory = () => run("history", async () => {
    historyOpen = !historyOpen;
    if (historyOpen) history = await invoke<Changeset[]>("share_history", { instanceId: inst.id });
  });
  const stop = () => run("stop", async () => { await invoke("share_stop", { instanceId: inst.id }); confirmStop = false; code = ""; await load(); await syncInstances(); });

  const VERB = { added: "Added", removed: "Removed", changed: "Changed" };
  const when = (ms: number) => (ms ? new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }) : "never");
  const online = (m: Member) => Date.now() - m.lastSeenMs < 3 * 60_000;
</script>

<div class="back" role="presentation" onclick={() => !busy && close()}>
  <div class="dialog" role="dialog" aria-modal="true" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && close()}>
    <button class="x" onclick={close} aria-label="Close">✕</button>
    <h3>Collaborate · {inst.name}</h3>

    {#if !st}
      <p class="muted">Loading…</p>
    {:else if !st.shared}
      <p>Play this pack with friends and keep it in step: when anyone adds or removes mods or changes configs, the others get those changes to review and apply.</p>
      <ul class="muted small">
        <li>Obelisk connects straight to your friends' Obelisk, encrypted. No account and no server of ours.</li>
        <li>Changes travel when you're both online with Obelisk open.</li>
        <li>Worlds and your personal game settings aren't shared.</li>
        <li>Every applied change can be undone from Backups.</li>
        <li>You get a short code. Your friend pastes it into Import, and Obelisk downloads the pack and sets it up to collaborate.</li>
        <li>You can turn collaboration off here any time.</li>
      </ul>
      <div class="actions"><div class="spacer"></div><button class="ghost" onclick={close}>Not now</button><button class="primary" disabled={!!busy} onclick={start}>{busy ? "Making it…" : "Make it collaborative"}</button></div>
    {:else if st.share}
      <section>
        <div class="row">
          <strong>Invite a friend</strong>
          <span class="muted small">They paste the code into Import, like a Share code.</span>
          <div class="spacer"></div>
          {#if !(code && left)}<button class="ghost" disabled={!!busy} onclick={invite}>{busy === "invite" ? "Making a code…" : "Get a code"}</button>{/if}
        </div>
        {#if code && left}
          <div class="row"><button class="short mono" title="Copy the code" onclick={copy}>{code}</button>
          <button class="copy-btn" class:done={copied} onclick={copy} aria-label="Copy the code">
            {#if copied}✓ Copied{:else}<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path d="M9 9h10v11H9zM5 15V4h10" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" /></svg> Copy{/if}
          </button><span class="muted small">Expires in {clock}. Keep Obelisk open until your friend shows up under Members.</span></div>
        {/if}
      </section>

      <section>
        <div class="row"><strong>Members</strong><div class="spacer"></div><button class="ghost" disabled={!!busy} onclick={sync}>{busy === "sync" ? "Checking…" : "Check for changes now"}</button></div>
        <div class="members">
          {#each st.share.members as m (m.id)}
            <span class="member" title={m.id}><i class:on={m.id === st.me || online(m)}></i>{m.name || "Friend"}{#if m.id === st.owner}&nbsp;<small>(owner)</small>{/if}{#if m.id === st.me}&nbsp;(you){:else}<small> · seen {when(m.lastSeenMs)}</small>{/if}
              {#if isOwner && m.id !== st.me}<button class="rm" title="Remove {m.name || 'them'} from this pack" aria-label="Remove {m.name || 'them'}" onclick={() => (confirmRemove = m)}>✕</button>{/if}</span>
          {/each}
        </div>
        {#if confirmRemove}
          <div class="row confirm">
            <span class="small">Remove <b>{confirmRemove.name || "this friend"}</b> from {inst.name}? Their copy stops getting changes and turns into a normal pack, and they'll see that you removed them. Invite codes you gave out before stop working.</span>
            <div class="spacer"></div>
            <button class="ghost" onclick={() => (confirmRemove = null)}>Cancel</button>
            <button class="primary danger" disabled={!!busy} onclick={() => remove(confirmRemove!)}>{busy === "remove" ? "Removing…" : "Remove"}</button>
          </div>
        {/if}
      </section>

      <section>
        <div class="row">
          <strong>Your changes</strong>
          <span class="muted small">{st.local.length ? `${st.local.length} not sent yet` : "Nothing new"}</span>
          <div class="spacer"></div>
          {#if st.local.length}
            <input class="search" placeholder="What did you change? (optional)" bind:value={label} />
            <button class="primary" disabled={!!busy} onclick={send}>Send to friends</button>
          {/if}
        </div>
        {#if st.local.length}
          <ul class="changes">{#each st.local.slice(0, 6) as c (c.path)}<li><span class="k {c.kind}">{VERB[c.kind]}</span> {c.label}</li>{/each}{#if st.local.length > 6}<li class="muted">…and {st.local.length - 6} more</li>{/if}</ul>
        {/if}
      </section>

      <section>
        <div class="row">
          <strong>From friends</strong>
          <span class="muted small">{st.incoming.length ? `${st.incoming.length} waiting` : "All caught up"}</span>
          <div class="spacer"></div>
          {#if st.incoming.length > 1}<button class="primary" disabled={!!busy} onclick={() => apply([])}>Apply all</button>{/if}
        </div>
        <div class="incoming">
          {#each st.incoming as c (c.id)}
            <div class="cs">
              <button class="head" onclick={() => (open = open === c.id ? null : c.id)}>
                <span class="caret">{open === c.id ? "▾" : "▸"}</span>
                <strong>{c.authorName}</strong> <span class="muted small">{when(c.atMs)}</span>
                <span class="what">{c.label || (c.changes.length ? `${c.changes.length} change${c.changes.length === 1 ? "" : "s"}` : "The whole pack")}</span>
              </button>
              {#if open === c.id}
                <ul class="changes body">{#each c.changes as ch (ch.path)}<li><span class="k {ch.kind}">{VERB[ch.kind]}</span> {ch.label}</li>{:else}<li class="muted">The full pack as they shared it.</li>{/each}</ul>
              {/if}
              <div class="row cs-actions"><div class="spacer"></div>
                <button class="ghost" disabled={!!busy} onclick={() => skip(c.id)} title="Keep your version; this won't be offered again">Skip</button>
                <button class="primary" disabled={!!busy} onclick={() => apply([c.id])}>Apply</button>
              </div>
            </div>
          {/each}
        </div>
        <label class="row small"><input type="checkbox" checked={st.share.autoApply} onchange={(e) => setAuto(e.currentTarget.checked)} /> Apply friends' changes automatically (still backed up first)</label>
        <label class="row small"><input type="checkbox" checked={st.share.autoSend ?? false} onchange={(e) => setAutoSend(e.currentTarget.checked)} /> Send my changes automatically (once they've settled, never while the game runs)</label>
      </section>

      {#if st.held?.length}
        <section class="held">
          <div class="row"><strong>Blocked files</strong><span class="muted small">Obelisk kept these out: they act like programs that steal accounts or passwords</span></div>
          <ul class="changes">
            {#each st.held as h (h.path)}
              <li><span class="k removed">Blocked</span><span class="hp"><b>{h.path.split("/").pop()}</b> <span class="muted small">from {h.from || "a friend"}</span><br /><span class="small">It {h.reasons.join(", and it ")}.</span></span>
                <div class="spacer"></div><button class="ghost" disabled={!!busy} onclick={review}>Review…</button></li>
            {/each}
          </ul>
          <span class="muted small">Some real mods do this for their own sign-in features. Allow one only if you know it and trust where it came from.</span>
        </section>
      {/if}

      <section>
        <div class="row"><strong>History</strong><span class="muted small">Who changed what, newest first</span><div class="spacer"></div>
          <button class="ghost" disabled={busy === "history"} onclick={showHistory}>{historyOpen ? "Hide" : "Show"}</button></div>
        {#if historyOpen && history}
          <div class="incoming">
            {#each history as c (c.id)}
              <div class="cs">
                <button class="head" onclick={() => (open = open === "h" + c.id ? null : "h" + c.id)}>
                  <span class="caret">{open === "h" + c.id ? "▾" : "▸"}</span>
                  <strong>{c.author === st.me ? "You" : c.authorName}</strong> <span class="muted small">{when(c.atMs)}</span>
                  <span class="what">{c.label || (c.changes.length ? `${c.changes.length} change${c.changes.length === 1 ? "" : "s"}` : "Shared the pack")}</span>
                </button>
                {#if open === "h" + c.id}
                  <ul class="changes body">{#each c.changes as ch (ch.path)}<li><span class="k {ch.kind}">{VERB[ch.kind]}</span> {ch.label}</li>{:else}<li class="muted">The full pack as it was shared.</li>{/each}</ul>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </section>

      <div class="actions">
        {#if confirmStop}
          <span class="muted small">Turn collaboration off? Your pack stays as it is; friends keep their copies.</span>
          <div class="spacer"></div>
          <button class="ghost" onclick={() => (confirmStop = false)}>Cancel</button>
          <button class="primary danger" disabled={!!busy} onclick={stop}>Turn off</button>
        {:else}
          <label class="row small"><input type="checkbox" checked onchange={(e) => { e.currentTarget.checked = true; confirmStop = true; }} /> Collaboration on</label>
          <div class="spacer"></div>
          <button class="primary" onclick={close}>Done</button>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; background: rgb(0 0 0 / 0.6); display: grid; place-items: center; z-index: 40; }
  .dialog { position: relative; width: min(720px, 94vw); max-height: 90vh; overflow-y: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 10px; padding: 22px 26px; display: flex; flex-direction: column; gap: 14px; outline: none; }
  .x { position: absolute; top: 10px; right: 10px; background: none; border: 0; color: var(--muted); padding: 4px 8px; }
  h3 { margin: 0; font-size: 17px; }
  p { margin: 0; }
  ul { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 4px; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  section { display: flex; flex-direction: column; gap: 8px; border-top: 1px solid var(--line); padding-top: 12px; }
  .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .spacer { flex: 1; }
  .code { width: 100%; height: 64px; resize: none; font: 11.5px ui-monospace, Consolas, monospace; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px; word-break: break-all; }
  .short { font: 700 22px Consolas, ui-monospace, monospace; letter-spacing: 3px; background: none; border: 0; padding: 0; color: inherit; cursor: copy; }
  .copy-btn { display: inline-flex; align-items: center; gap: 6px; background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 6px 12px; font-weight: 600; flex: none; }
  .copy-btn.done { background: var(--ok, #2e9d5b); }
  .members { display: flex; flex-wrap: wrap; gap: 8px; }
  .rm { margin-left: 6px; padding: 0 4px; border: 0; background: none; color: var(--muted); font-size: 12px; }
  .rm:hover { color: var(--err-text); }
  .confirm { margin-top: 8px; padding: 8px 10px; border-radius: 6px; background: var(--panel-2); }
  .member { display: inline-flex; align-items: center; gap: 6px; background: var(--panel); border: 1px solid var(--line); border-radius: 999px; padding: 4px 12px; font-size: 13px; }
  .member i { width: 8px; height: 8px; border-radius: 50%; background: var(--muted); }
  .member i.on { background: var(--mr); }
  .changes { list-style: none; padding: 0; font-size: 13px; }
  .changes li { display: flex; align-items: center; gap: 8px; }
  .held .changes li { align-items: flex-start; }
  .hp { min-width: 0; overflow-wrap: anywhere; }
  .k { font-size: 11px; font-weight: 700; padding: 1px 6px; border-radius: 3px; min-width: 64px; text-align: center; }
  .k.added { background: color-mix(in srgb, var(--mr) 18%, transparent); color: var(--mr); }
  .k.removed { background: color-mix(in srgb, var(--danger) 18%, transparent); color: var(--danger); }
  .k.changed { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .incoming { display: flex; flex-direction: column; gap: 6px; }
  .cs { border: 1px solid var(--line); border-radius: 6px; background: var(--panel); padding: 4px 10px 8px; }
  .head { display: flex; align-items: center; gap: 8px; width: 100%; background: none; border: 0; padding: 6px 0; text-align: left; }
  .caret { width: 12px; color: var(--muted); }
  .what { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text-2); }
  .body { padding-left: 20px; }
  .cs-actions { margin-top: 4px; }
  .search { width: 240px; }
  .actions { display: flex; gap: 10px; align-items: center; border-top: 1px solid var(--line); padding-top: 12px; }
  .primary { color: #fff; background: var(--accent); border: 0; border-radius: 4px; padding: 7px 16px; font-weight: 600; }
  .primary:disabled { opacity: 0.6; }
  .primary.danger { background: var(--danger); }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 14px; }
</style>
