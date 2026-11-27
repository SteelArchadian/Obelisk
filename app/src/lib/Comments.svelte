<script lang="ts">
  // A CurseForge project's comments, newest first, replies under each. Signed in (Settings → CurseForge API, or the
  // bell), the user can comment and reply here; threads they're in are watched for replies (the bell).
  import { invoke } from "@tauri-apps/api/core";
  import { ago } from "./api";
  import { cleanHtml, wireYouTube } from "./richtext";
  import CommentBox from "./CommentBox.svelte";
  import { cf, cfSignIn } from "./account.svelte";
  import { toastError } from "./state.svelte";

  type Author = { id: number; username: string; name: string; avatar?: string };
  type Comment = { id: number; html: string; posted: number; pinned: boolean; author: Author; replies?: Comment[] };

  let { projectId, projectName }: { projectId: number; projectName: string } = $props();

  let comments = $state<Comment[]>([]);
  let total = $state(0);
  let enabled = $state<boolean | null>(null); // false: the author turned comments off on CurseForge
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
      const v = await invoke<{ comments: Comment[]; total: number; enabled?: boolean | null }>("cf_comments", { projectId, projectName, page: p });
      comments = p === 0 ? v.comments : [...comments, ...v.comments.filter((c) => !comments.some((x) => x.id === c.id))];
      total = v.total ?? comments.length;
      if (p === 0) enabled = v.enabled ?? null;
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

  async function post(html: string, parentId: number | null) {
    posting = true;
    try {
      await invoke("cf_comment_post", { projectId, projectName, html, parentId });
      if (parentId === null) draft = "";
      else { replyDraft = ""; replyTo = null; }
      await load(0);
    } catch (e) {
      toastError(e);
    }
    posting = false;
  }

  // Quote a comment into the box (the reply box under its thread, or the main box).
  function quote(c: Comment, threadId: number | null) {
    const text = (new DOMParser().parseFromString(cleanHtml(c.html), "text/html").body.innerText || "").trim();
    const who = c.author?.name || c.author?.username || "someone";
    const q = `> *Quote from **${who}**:*\n` + text.split("\n").map((l) => "> " + l).join("\n") + "\n\n";
    if (threadId === null) draft = q + draft;
    else { if (replyTo !== threadId) replyDraft = ""; replyTo = threadId; replyDraft = q + replyDraft; }
  }

  // Spoilers stay hidden until clicked; videos keep the remembered volume.
  function bodyClick(e: MouseEvent) {
    const sp = (e.target as HTMLElement).closest(".spoiler");
    if (sp && !sp.classList.contains("shown")) { e.preventDefault(); e.stopPropagation(); sp.classList.add("shown"); }
  }
  let listEl = $state<HTMLElement>();
  $effect(() => {
    comments;
    if (listEl) queueMicrotask(() => wireYouTube(listEl!));
  });
  // Pictures that didn't load; those authors (and ones with none) get their first letter instead.
  let broken = $state(new Set<string>());
  const hue = (s: string) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
  const mine = (c: Comment) => !!cf.account && c.author?.id === cf.account.id;
</script>

<section class="comments" bind:this={listEl} onclickcapture={bodyClick}>
  {#if enabled === false}
    <p class="muted">Comments are turned off for this project.</p>
  {:else if cf.account}
    <CommentBox bind:value={draft} busy={posting} placeholder={`Comment on ${projectName} as ${cf.account.displayName || cf.account.username}`} onsubmit={(html) => post(html, null)} />
  {:else}
    <div class="signin">
      <span class="muted">Sign in to CurseForge to comment and reply here.</span>
      <button class="ghost" onclick={cfSignIn}>{cf.signingIn ? "Show the sign-in window" : "Sign in"}</button>
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
            <CommentBox bind:value={replyDraft} busy={posting} rows={2} label="Reply" placeholder="Write a reply" autofocus onsubmit={(html) => post(html, c.id)} oncancel={() => { replyTo = null; replyDraft = ""; }} />
          </div>
        {:else}
          <div class="acts">
            <button class="link" onclick={() => { replyTo = c.id; replyDraft = ""; }}>Reply</button>
            <button class="link" onclick={() => quote(c, c.id)}>Quote</button>
          </div>
        {/if}
      {/if}
    </article>
  {:else}
    {#if !loading && !failed && enabled !== false}<p class="muted">No comments yet.</p>{/if}
  {/each}

  {#if loading}<p class="muted">Loading…</p>
  {:else if comments.length < total}<button class="ghost more" onclick={() => load(page + 1)}>Show older comments</button>{/if}
</section>

{#snippet one(c: Comment)}
  <div class="comment" class:mine={mine(c)}>
    {#if c.author?.avatar && !broken.has(c.author.avatar)}
      <img class="avatar" src={c.author.avatar} alt="" loading="lazy" onerror={() => (broken = new Set(broken).add(c.author.avatar!))} />
    {:else}
      {@const who = c.author?.name || c.author?.username || "?"}
      <div class="avatar letter" style:background={`hsl(${hue(who)} 45% 38%)`} aria-hidden="true">{who.trim().charAt(0).toUpperCase()}</div>
    {/if}
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
  .signin { display: flex; align-items: center; gap: 12px; background: var(--panel); padding: 10px 12px; border-radius: 8px; }
  .thread { display: flex; flex-direction: column; gap: 8px; border-bottom: 1px solid var(--line); padding-bottom: 12px; }
  .comment { display: flex; gap: 10px; }
  .comment.mine .main { border-left: 3px solid var(--accent); padding-left: 8px; }
  .avatar { width: 32px; height: 32px; border-radius: 50%; flex: none; background: var(--panel); object-fit: cover; }
  .letter { display: grid; place-items: center; color: #fff; font-weight: 700; font-size: 14px; }
  .main { min-width: 0; flex: 1; }
  .head { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
  .tag { font-size: 11px; padding: 1px 6px; border-radius: 999px; background: var(--panel); color: var(--muted); }
  .body { margin-top: 2px; overflow-wrap: anywhere; }
  .comments :global(.body p) { margin: 4px 0; }
  .comments :global(.body img) { max-width: 100%; height: auto; }
  .comments :global(.body iframe) { width: 100%; max-width: 560px; aspect-ratio: 16 / 9; height: auto; border: 0; border-radius: 6px; }
  .comments :global(.body blockquote) { margin: 6px 0; padding: 6px 10px; border-left: 3px solid var(--line); background: var(--panel); border-radius: 4px; }
  .comments :global(.body .quote-source) { font-size: 12px; color: var(--muted); margin-bottom: 4px; }
  .comments :global(.body pre) { background: var(--panel); padding: 8px 10px; border-radius: 4px; overflow-x: auto; white-space: pre-wrap; }
  .comments :global(.body code) { background: var(--panel); padding: 1px 4px; border-radius: 3px; font-size: 0.92em; }
  .comments :global(.body pre code) { background: none; padding: 0; }
  .comments :global(.body ul), .comments :global(.body ol) { margin: 4px 0; padding-left: 22px; }
  .comments :global(.body h1), .comments :global(.body h2), .comments :global(.body h3), .comments :global(.body h4) { margin: 8px 0 4px; font-size: 1.05em; }
  .comments :global(.body table) { border-collapse: collapse; }
  .comments :global(.body td), .comments :global(.body th) { border: 1px solid var(--line); padding: 3px 6px; }
  .comments :global(.body .spoiler:not(.shown)) { background: var(--muted); color: transparent; border-radius: 3px; cursor: pointer; user-select: none; }
  .comments :global(.body .spoiler:not(.shown) *) { visibility: hidden; }
  .reply { margin-left: 42px; }
  .acts { display: flex; gap: 14px; margin-left: 42px; font-size: 13px; }
  .more { align-self: center; }
  .muted { color: var(--muted); }
  .warn { color: var(--warn, #e0a040); }
  .ghost { background: none; border: 1px solid var(--line); border-radius: 4px; padding: 7px 12px; color: inherit; }
  .link { background: none; border: 0; color: var(--accent); padding: 0; }
</style>
