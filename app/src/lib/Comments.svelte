<script lang="ts">
  // A CurseForge project's comments, newest first, replies under each. Signed in (Settings → CurseForge API, or the
  // bell), the user can comment and reply here; threads they're in are watched for replies (the bell).
  import { invoke } from "@tauri-apps/api/core";
  import { ago } from "./api";
  import { cleanHtml } from "./richtext";
  import { cf, cfSignIn } from "./account.svelte";
  import { toastError } from "./state.svelte";

  type Author = { id: number; username: string; name: string; avatar?: string };
  type Comment = { id: number; html: string; posted: number; pinned: boolean; author: Author; replies?: Comment[] };

  let { projectId, projectName }: { projectId: number; projectName: string } = $props();

  let comments = $state<Comment[]>([]);
  let total = $state(0);
  let page = $state(0);
  let loading = $state(false);
  let failed = $state("");
  let draft = $state("");
  let replyTo = $state<number | null>(null);
  let replyDraft = $state("");
  let posting = $state(false);

  async function load(p: number) {
    loading = true;
    failed = "";
    try {
      const v = await invoke<{ comments: Comment[]; total: number }>("cf_comments", { projectId, projectName, page: p });
      comments = p === 0 ? v.comments : [...comments, ...v.comments.filter((c) => !comments.some((x) => x.id === c.id))];
      total = v.total ?? comments.length;
      page = p;
    } catch (e) {
      failed = String(e);
    }
    loading = false;
  }

  $effect(() => {
    projectId;
    cf.account;
    load(0);
  });

  async function post(text: string, parentId: number | null) {
    posting = true;
    try {
      await invoke("cf_comment_post", { projectId, projectName, text, parentId });
      if (parentId === null) draft = "";
      else { replyDraft = ""; replyTo = null; }
      await load(0);
    } catch (e) {
      toastError(e);
    }
    posting = false;
  }

  const focus = (el: HTMLElement) => el.focus();
  const mine = (c: Comment) => !!cf.account && c.author?.id === cf.account.id;
</script>

<section class="comments">
  {#if cf.account}
    <div class="compose">
      <textarea rows="3" placeholder={`Comment on ${projectName} as ${cf.account.displayName || cf.account.username}`} bind:value={draft} disabled={posting}></textarea>
      <button class="primary" disabled={!draft.trim() || posting} onclick={() => post(draft, null)}>{posting && replyTo === null ? "Posting…" : "Post"}</button>
    </div>
  {:else}
    <div class="signin">
      <span class="muted">Sign in to CurseForge to comment and reply here.</span>
      <button class="ghost" disabled={cf.signingIn} onclick={cfSignIn}>{cf.signingIn ? "Finish in the CurseForge window…" : "Sign in"}</button>
    </div>
  {/if}

  {#if failed}<p class="warn">{failed} <button class="link" onclick={() => load(0)}>Try again</button></p>{/if}

  {#each comments as c (c.id)}
    <article class="thread">
      {@render one(c)}
      {#each c.replies ?? [] as r (r.id)}
        <div class="reply">{@render one(r)}</div>
      {/each}
      {#if cf.account}
        {#if replyTo === c.id}
          <div class="compose reply">
            <textarea rows="2" placeholder="Write a reply" bind:value={replyDraft} disabled={posting} use:focus></textarea>
            <div class="row">
              <button class="ghost" onclick={() => { replyTo = null; replyDraft = ""; }}>Cancel</button>
              <button class="primary" disabled={!replyDraft.trim() || posting} onclick={() => post(replyDraft, c.id)}>{posting ? "Posting…" : "Reply"}</button>
            </div>
          </div>
        {:else}
          <button class="link reply-btn" onclick={() => { replyTo = c.id; replyDraft = ""; }}>Reply</button>
        {/if}
      {/if}
    </article>
  {:else}
    {#if !loading && !failed}<p class="muted">No comments yet.</p>{/if}
  {/each}

  {#if loading}<p class="muted">Loading…</p>
  {:else if comments.length < total}<button class="ghost more" onclick={() => load(page + 1)}>Show older comments</button>{/if}
</section>

{#snippet one(c: Comment)}
  <div class="comment" class:mine={mine(c)}>
    {#if c.author?.avatar}<img class="avatar" src={c.author.avatar} alt="" loading="lazy" />{:else}<div class="avatar"></div>{/if}
    <div class="main">
      <div class="head">
        <strong>{c.author?.name || c.author?.username || "Someone"}</strong>
        {#if c.pinned}<span class="tag">Pinned</span>{/if}
        {#if mine(c)}<span class="tag">You</span>{/if}
        <small class="muted">{ago(c.posted)}</small>
      </div>
      <div class="rich body">{@html cleanHtml(c.html)}</div>
    </div>
  </div>
{/snippet}

<style>
  .comments { display: flex; flex-direction: column; gap: 14px; padding-top: 6px; }
  .compose { display: flex; flex-direction: column; gap: 8px; }
  .compose textarea { width: 100%; resize: vertical; background: var(--panel); color: var(--text); border: 1px solid var(--line); border-radius: 6px; padding: 8px 10px; font: inherit; }
  .compose > .primary { align-self: flex-end; }
  .row { display: flex; gap: 8px; justify-content: flex-end; }
  .signin { display: flex; align-items: center; gap: 12px; background: var(--panel); padding: 10px 12px; border-radius: 8px; }
  .thread { display: flex; flex-direction: column; gap: 8px; border-bottom: 1px solid var(--line); padding-bottom: 12px; }
  .comment { display: flex; gap: 10px; }
  .comment.mine .main { border-left: 3px solid var(--accent); padding-left: 8px; }
  .avatar { width: 32px; height: 32px; border-radius: 50%; flex: none; background: var(--panel); object-fit: cover; }
  .main { min-width: 0; flex: 1; }
  .head { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .tag { font-size: 11px; padding: 1px 6px; border-radius: 999px; background: var(--panel); color: var(--muted); }
  .body { margin-top: 2px; overflow-wrap: anywhere; }
  .body :global(p) { margin: 4px 0; }
  .body :global(img) { max-width: 100%; }
  .reply { margin-left: 42px; }
  .reply-btn { align-self: flex-start; margin-left: 42px; font-size: 13px; }
  .more { align-self: center; }
  .muted { color: var(--muted); }
  .warn { color: var(--warn, #e0a040); }
  .primary { background: var(--accent); color: #fff; border: 0; border-radius: 4px; padding: 7px 14px; font-weight: 600; }
  .primary:disabled { opacity: 0.6; }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 12px; color: inherit; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; }
</style>
